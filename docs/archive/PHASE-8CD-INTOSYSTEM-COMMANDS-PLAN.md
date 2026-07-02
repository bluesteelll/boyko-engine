# Architecture: Phase 8c + 8d — `IntoSystem`/`FunctionSystem` + `Commands` deferred mutation buffer

## Changes from Round 3 (Round 4 editorial cleanup pass)

This is **Round 4**, addressing the architecture-critic's Round 3 verdict, which was "CHANGES REQUESTED" with **semantic content APPROVED, presentation not**. No design decisions reopened — only editorial cleanups.

| Item | Status | Where addressed |
|------|--------|-----------------|
| **C1''** §12.1 contained two contradictory `Bundle::for_each_component_bytes` signatures (FnOnce → FnMut "re-checking") with first-person reasoning prose between them | **FIXED** — deleted the duplicate FnOnce signature + "Actually re-checking" + "Resolution (C3' final)" prose. Single clean trait body with `F: FnMut(ComponentId, &[u8])` (final, Round 4). The N >= 2 invocations require `FnMut`; this is the minimal bound. §12.1. |
| **C2''** §10.3 contained a tautological `debug_assert!(self.bytes.is_empty() \|\| !self.bytes.is_empty(), "placeholder")` | **FIXED** — deleted the placeholder debug_assert. The `panic_recovery` Bevy invariant assertion immediately following stays. §10.3. |
| **W1''** §10.4 `consume_and_drop_glue` lacked an explicit statement of single-drop guarantee on panic-during-apply | **FIXED** — added "Drop-during-unwind / single-drop guarantee" doc paragraph. Local `cmd` drops once via unwind drop-elaboration; byte slot left moved-from; cursor already advanced past the panicker (W3'); survivors drop via recovery redrive. No double-drop, no leak. §10.4. |
| **W2''** §12.4 MaybeUninit→T slice cast had a SAFETY block but missed enumerating the layout-compatibility argument, and lacked a dedicated Miri test | **FIXED** — replaced SAFETY with 4-bullet enumerated block (init by `.write()`, MaybeUninit/T layout compat per Rust reference, `count` exact, lifetimes). Added Miri test `miri_bundle_slice_cast_arity_1_and_4` in §Step 12. §12.4. |
| **W3''** §12.4 used a `_entity_holder: Option<Entity>` workaround variable + `entity_created` flag — code smell | **FIXED** — eliminated both. The Entity return is discarded via `let _ = world.create_entity(...)` (Phase 8d does not surface entity ID; Phase 11 will add `SpawnCommandReturning<B>`). Post-callback invariant check uses `debug_assert_eq!(count, arity, ...)`. §12.4. |
| **O1''** Round 3 change log row for C3' said "All `FnMut` → `FnOnce`"; final bound is `FnMut` | **FIXED** — see C1'' entry above; C3' row in §"Changes from Round 2" updated to reflect the final `FnMut` bound below. |

## Changes from Round 2

This is **Round 3** following architecture-critic's CHANGES REQUESTED verdict on Round 2. Highlights:

| Item | Status | Where addressed |
|------|--------|-----------------|
| **C1'** `for_each_component_bytes` panic-safety hole — `mem::forget` skipped on callback panic ⇒ double drop UB | **FIXED** — adopted ManuallyDrop-from-the-start pattern; each tuple element wrapped in `ManuallyDrop<T>` before any callback runs. On panic mid-iteration, the unfinished elements leak (no Drop runs), no double-drop. Documented as new invariant **B4** (panic-leak < UB). Step 12 Miri test `miri_bundle_for_each_panics_no_double_drop` added. §12.3. |
| **C2'** `CommandQueue::apply` "Step 0.5" prepended panic_recovery to bytes — diverged from Bevy semantics | **FIXED** — **deleted Step 0.5 entirely**. Bevy's apply NEVER reads panic_recovery at the start; recovery is OPAQUE between applies until the catch_unwind Err branch's top-level (start == 0) absorb step transitions it back into bytes. Step 8 acceptance test rewritten to verify Bevy's actual redrive cycle: a top-level panic absorbs the unrun tail into bytes on the SAME apply call (via the Err branch), and the NEXT apply call walks those redriven bytes. §10.3 + §21.2 + §29 Q3 + Step 8 updated. |
| **C3'** §12.4 / §12.3 had draft scaffolding interleaved with final code (multiple `unreachable!` blocks, first-person prose, dead `static IDS:` block, FnMut vs FnOnce mismatch) | **FIXED** — single canonical code block per section. `static IDS:` removed (was dead code that the `bundle_slot_for` helper subsumes). Bundle apply callback bound: **`F: FnMut`** (final, Round 4 — `FnMut` is required for N >= 2 invocations; see C1''). `ArchetypeRow` decision finalized — see W4'. §12.3 + §12.4 rewritten. |
| **W1'** `CommandQueue` / `RawCommandQueue` Send/Sync derivation unspecified | **FIXED** — `unsafe impl Send for CommandQueue` documented + justified; `RawCommandQueue` remains `!Send + !Sync` (NonNull raw ptrs). Phase 9 may revisit RawCommandQueue's bounds when the scheduler is designed. §10.1 + §19. |
| **W2'** §4.7 reproducer didn't test fully-elided closure-arg inference | **FIXED** — added a THIRD reproducer test `closure_with_elided_param_type_compiles` exercising `|p: StubParam|` (no `<'_, '_>` annotation). If this fails, the §4.7.1 fallback documents `|p: StubParam<'_, '_>|` as the user-facing form. §4.7 + Step 2 acceptance amended. |
| **W3'** Panic-mid-apply cursor advancement: retry the panicked command, or skip? | **FIXED** — **skip**. Matches Bevy: cursor advancement happens BEFORE `consume_and_drop` is called, so the panicked command is NOT in `bytes[local_cursor..]` when the Err branch runs. Recovery captures the *remaining* commands (after the panicker). §10.3 + §10.4 made explicit; Step 8 acceptance test `command_queue_panic_skips_panicker_runs_rest_on_redrive` added. |
| **W4'** `ArchetypeRow` was opaque, undefined | **FIXED** — `ArchetypeRow` **dropped entirely**. SpawnCommand::apply now uses a stack-allocated `[(ComponentId, &[u8]); 4]` collector inside `for_each_component_bytes`, then calls existing `EcsMaster::create_entity(archetype_id, &slots[..count])`. No new EcsMaster API surface. Re-traced lifetime soundness: the bundle's destructured locals live until `for_each_component_bytes` returns; the callback runs inside the function; the collector array's slices reference those locals; `create_entity` is called AFTER `for_each_component_bytes` returns — **BUT** the slices in the collector now reference dropped/forgotten memory. The corrected pattern threads `create_entity` INSIDE the callback's terminal invocation. See §12.4. |
| **W5'** Dead `static IDS:` block in §12.3 | **FIXED** — Same as C3'. Block removed. |
| **O1'** `Box::leak`-per-bundle-type memory cost | **DOCUMENTED** — bounded by `N_BUNDLE_TYPES × ~16 B` (one leaked `[ComponentId; N]`); typical project < 100 bundle types ⇒ < 2 KB. Acceptable. §12.3. |
| **O2'** Add "first-call total" row to §1.2 | **ADDED** — new row `EcsMaster::run_closure_once` first-call (cold init+state alloc+run+apply) ≈ 1.2 µs. §1.2. |
| **O3'** Confirm `SystemParam::apply` and `System::apply` are both safe `fn apply` | **DOCUMENTED** — both are SAFE `fn apply(&mut self, world: &mut EcsMaster)`. No `unsafe` on either method. Caller already has `&mut EcsMaster` exclusively; no aliasing risk. §9.5 + §18 APP1' note. |

The C2' deletion changes the apply semantics meaningfully — verify Open Q3 + Step 8 acceptance reads correctly before implementation.

## Changes from Round 1
(Retained for traceability; see prior section.)

| Item | Status | Where addressed |
|------|--------|-----------------|
| **C1** Bundle::write_into transmute<&[u8], &'static [u8]> unsound | **FIXED** in Round 2 — callback API. (Round 3 refines the panic-safety side per C1'.) |
| **C2** Component vs Bundle: Send blanket — silent breakage | **DECIDED** in Round 2 — `impl<C: Component + Send + Sync> Bundle for (C,)`. |
| **C3** ApplyGuard raw pointer aliases live &mut — Tree Borrows UB | **FIXED** in Round 2 — RawCommandQueue pattern. |
| **C4** Packed<T> repr(C, packed) reference creation UB risk | **FIXED** in Round 2 — offset constants. |
| **C5** panic_recovery drain semantics unspecified | **FIXED in Round 2 with Step 0.5 prepend** → **REVISED in Round 3 (C2'): Step 0.5 deleted; pure Bevy mirror.** |
| **C6** Two-lifetime GAT HRTB reproducer unverified | **VERIFIED (positive)** in Round 2. Round 3 adds fully-elided test (W2'). |
| **W1** Tuple SystemParam::apply ordering | **FIXED** in Round 2 — APP3 invariant + Step 10 test. |
| **W2** Command: Send + 'static rationale | **DOCUMENTED** in Round 2. |
| **W3** CommandQueue capacity unbounded | **DECIDED** in Round 2 — policy (a). |
| **W4** Phase 8a tuple_impl const-panic latent bug | **VERIFIED-then-DEFER** in Round 2. |
| **W5** run_closure_once perf claim split | **FIXED** in Round 2. |
| **W6** run_closure_once Send+Sync diagnostic | **FIXED** in Round 2. |
| **W7** Bundle::component_ids OnceLock impl missing | **FIXED** in Round 2; Round 3 strips dead scaffolding (C3'). |
| **W8** System::apply re-entrancy safety contract | **FIXED** in Round 2 — APP4. |
| **O1-O5** | All fixed in Round 2. |

---

## 1. Goal and target metrics

### 1.1 Goal

Deliver the **two final ergonomic pillars** completing Phase 8's system API surface, retiring the `FnOnceSystem` stub from 8a and unblocking real-game-loop usage:

* **8c — `IntoSystem` + `FunctionSystem<F, M>`.** Convert any `fn(P0, P1, ..., Pn)` (with `Pi: SystemParam`) into a runnable `System` without turbofish at the call site. Cache `<F::Param as SystemParam>::State` + `SystemMeta` ACROSS invocations (Phase 8a's `FnOnceSystem` rebuilds per call, paying ~960 ns dispatch).
* **8d — `Commands` SystemParam.** Per-system byte-arena queue for deferred mutations (`spawn`, `despawn`, generic `add<C: Command>`). Flushed via `SystemParam::apply` after the system body returns. No `Box<dyn Command>`, no allocation per command (memcpy into the queue's `Vec<MaybeUninit<u8>>`).

Exit criteria for 8c+8d (10 items unchanged from Round 1).

### 1.2 Target metrics — 8c (release, AMD Zen3 / Intel Alder Lake) — **W5 split + O2' first-call row**

| Operation | Target | Cache profile |
|-----------|--------|---------------|
| `FunctionSystem::run_unsafe` empty-param call | ≤ 5 ns dispatch overhead | call + return |
| `FunctionSystem::run_unsafe` with 1× `Res<T>` param | ≤ 8 ns (5 ns dispatch + 3 ns `Res::get_param`) | 1 L1d hit |
| `FunctionSystem::run_unsafe` with `(Query<&A>, Res<B>)` | dominated by Query's archetype refresh; per-system add ≤ 3 ns | per Phase 8b |
| `FunctionSystem::initialize` (cold; once per system lifetime) | ≤ 1 µs | per Phase 8a Step 8 |
| **`FunctionSystem::run_unsafe` HOISTED reuse** (W5) | **≤ 30 ns total per call after first init** | state cached, no rebuild |
| **`EcsMaster::run_closure_once` per-call** (W5) | **~960 ns (same as Phase 8a)** — rebuilds FunctionSystem each call | unchanged from 8a |
| **`EcsMaster::run_closure_once` FIRST-call cumulative** (O2') | **≈ 1.2 µs** (≤ 1 µs initialize + ≤ 30 ns dispatch + closure body + apply) | cold |

The 8c win is **only realised** when the user **hoists** the `FunctionSystem` outside their loop. `run_closure_once` is a test/dev convenience and explicitly documented as not the production path.

### 1.3 Target metrics — 8d (release)

| Operation | Target | Cache profile |
|-----------|--------|---------------|
| `Commands::spawn((Position,))` enqueue | ≤ 20 ns | 1 cache line (queue tail) |
| `Commands::despawn(entity)` enqueue | ≤ 15 ns | 1 cache line |
| `CommandQueue::apply_one` (per command) | ≤ 200 ns (`create_entity` + bundle write) | 2-3 cache lines |
| `CommandQueue::apply` empty queue | ≤ 3 ns | 1 line |
| `Commands::get_param` per system invocation | ≤ 3 ns | 0 cache misses |

### 1.4 Cross-phase relation
(Unchanged.)

---

## 2. Context and constraints

### 2.1 Subsystems affected
(Unchanged from Round 1.)

### 2.2 Invariants preserved
All Phase 7 (U1..U14), Phase 8a (R1..R5, U_C1..U_C3, SP1..SP4, S1, AB-R1), Phase 8b (QD1..QD4, QF1..QF2, Q1..Q5, QS1) invariants stand unchanged.
`EcsMaster: !Send + !Sync`. `UnsafeEcsCell: !Send + !Sync`.

### 2.3 New invariants introduced
(Full list in §18. Highlights:)
- **FS1** FunctionSystem state idempotence
- **FS2** FunctionSystem state reuse
- **FS3** SystemParamFunction HRTB inference contract
- **CQ1**..**CQ7** CommandQueue/Command invariants (revised; see §10 and §11)
- **CQ-PACK1** No reference creation into packed byte layout — use offset constants + memcpy
- **CQ-SEND1** `unsafe impl Send for CommandQueue` (W1')
- **B1, B2** Bundle canonical ordering + write contract
- **B3** Bundle Send/Sync transitive bound
- **B4** **NEW (C1')** Bundle::for_each_component_bytes panic-safety: on callback panic mid-iteration, unfinished components LEAK (no Drop, no double-drop) — leak < UB
- **APP1**..**APP4** System::apply contract
- **APP1'** **NEW (O3')** Both `System::apply` and `SystemParam::apply` are SAFE `fn apply(&mut self, world: &mut EcsMaster)` — caller already holds exclusive `&mut`; no aliasing risk

### 2.3.5 C2 RESOLUTION — Bundle vs Component Send requirement
(Unchanged from Round 2.)

### 2.4 Hard prohibitions
(Unchanged.)

### 2.5 Variadic arity ceiling
`MAX_SYSTEM_PARAM_FN_ARITY = 12`. Stubs cover 13..=24 with **runtime `panic!()`** (NOT `const { panic!() }`).

#### 2.5.1 W4 RESOLUTION — Phase 8a tuple_impl_too_large is sound; left alone
(Unchanged from Round 2.)

---

## 3. Decision C1 — `IntoSystem` trait shape
(Unchanged from Round 2.)

```rust
pub trait IntoSystem<In, Out, Marker>: Sized {
    type System: System<Out = Out>;
    fn into_system(self) -> Self::System;
}
```

---

## 4. Decision C2 — `SystemParamFunction<Marker>` trait shape (double-`FnMut` bound)

### 4.1 Trait declaration
(Unchanged.)

### 4.2 Variadic blanket impl
(Unchanged.)

### 4.3-4.6 (Unchanged.)

### 4.7 C6 VERIFICATION — Two-lifetime GAT HRTB reproducer (W2' added third test)

**Research findings:** (unchanged from Round 2 — Bevy uses identical two-lifetime `SystemParamItem<'w, 's, P>` GAT with lifetime-elided `SystemParamItem<$param>` form under `for<'a> &'a mut Func` HRTB; stable Rust 1.79+; boyko targets Rust 2024 ⇒ rustc ≥ 1.85.)

**Reproducer (paste into `crates/boyko_ecs/tests/hrtb_reproducer.rs`; Step 2 acceptance gate):**

```rust
//! Minimal HRTB reproducer for the C6 architecture decision.
//!
//! Validates that rustc infers the closure's parameter type without
//! turbofish when SystemParam::Item has TWO lifetimes (mirrors boyko's
//! `Query<'w, 's, D>` shape).

// === The trait surface (mirrors boyko's SystemParam + SystemParamFunction) ===

trait MyParam: Sized {
    type State: Send + Sync + 'static;
    type Item<'w, 's>: MyParam<State = Self::State>;
    fn get_param<'w, 's>(state: &'s mut Self::State) -> Self::Item<'w, 's>;
}

type ParamItem<'w, 's, P> = <P as MyParam>::Item<'w, 's>;

trait MyFn<Marker>: Send + Sync + 'static {
    type Param: MyParam;
    type Out;
    fn run(&mut self, p: ParamItem<'_, '_, Self::Param>) -> Self::Out;
}

impl<Out, Func, P> MyFn<fn(P) -> Out> for Func
where
    Func: Send + Sync + 'static,
    Out: 'static,
    P: MyParam,
    for<'a> &'a mut Func:
        FnMut(P) -> Out
      + FnMut(ParamItem<'_, '_, P>) -> Out,
{
    type Param = P;
    type Out = Out;
    fn run(&mut self, p: ParamItem<'_, '_, P>) -> Out {
        fn call_inner<Out, P>(mut f: impl FnMut(P) -> Out, p: P) -> Out { f(p) }
        call_inner(self, p)
    }
}

struct StubParam<'w, 's> { _w: std::marker::PhantomData<&'w ()>, _s: std::marker::PhantomData<&'s mut ()> }
struct StubState;
unsafe impl Send for StubState {}
unsafe impl Sync for StubState {}

impl<'a, 'b> MyParam for StubParam<'a, 'b> {
    type State = StubState;
    type Item<'w, 's> = StubParam<'w, 's>;
    fn get_param<'w, 's>(_s: &'s mut Self::State) -> Self::Item<'w, 's> {
        StubParam { _w: std::marker::PhantomData, _s: std::marker::PhantomData }
    }
}

fn run_closure<F, Out, Marker>(_body: F) -> Out
where
    F: MyFn<Marker>,
    Out: Default,
    Marker: 'static,
{
    Out::default()
}

// === Acceptance tests ===

#[test]
fn closure_compiles_without_turbofish() {
    // Test 1 — explicit two-lifetime annotation.
    let _: () = run_closure(|_p: StubParam<'_, '_>| {});
}

#[test]
fn function_pointer_compiles_without_turbofish() {
    fn body(_p: StubParam<'_, '_>) {}
    let _: () = run_closure(body);
}

#[test]
fn closure_with_elided_param_type_compiles() {
    // Test 3 (W2' NEW) — FULLY ELIDED. No `<'_, '_>` on `StubParam`.
    // This is the canonical user-facing form: `|q: Query<&Position>|`.
    // If this compiles, the Phase 8c headline ergonomic promise holds.
    // If it FAILS, halt and invoke §4.7.1 fallback (user must write
    // `Query<'_, '_, &Position>` or use a `lifetimeless::SQuery` alias).
    let _: () = run_closure(|_p: StubParam| {});
}
```

**Expected result:** All three tests compile and pass with rustc 1.85+ on stable. If `closure_with_elided_param_type_compiles` fails, the fallback in §4.7.1 documents the residual ergonomic gap.

**Status:** PROCEED. Step 2 of the implementation plan (§24) treats all three tests as the gate.

#### 4.7.1 Fallback if the reproducer fails

If rustc 1.85 cannot infer the closure's param without turbofish OR cannot elide both lifetimes (low probability — Bevy ships this; per Rust elision rules under HRTB, the binder fills in fresh anonymous lifetimes for both `'w` and `'s` per occurrence):

- **(a)** Accept `|p: StubParam<'_, '_>|` (or `|q: Query<'_, '_, &Position>|`) as the user-facing form. The two `'_` annotations are minimal noise. Document in `run_closure_once`'s rustdoc.
- **(b)** Add a `lifetimeless` alias module: `SQuery<D, F> = Query<'static, 'static, D, F>` with a custom `SystemParam` impl re-binding lifetimes during `get_param`. Mirrors Bevy's nightly-only `lifetimeless::SQuery`. Tracked as Phase 8c §C6 fallback.

Both are non-trivial — fallback is a Step 2 escape hatch, not a primary path. All three reproducer tests must compile before Step 4 (FunctionSystem) begins.

---

## 5. Decision C3 — `FunctionSystem<F, Marker>` struct shape
(Unchanged from Round 2.)

---

## 6. Decision C4 — `IntoSystem` blanket impls
(Unchanged from Round 2.)

---

## 7. Decision C5 — Variadic tuple impls + stubs
(Unchanged from Round 2.)

---

## 8. Decision C6 — Replace `FnOnceSystem` and `run_closure_once`
(Unchanged from Round 2.)

---

## 9. Decision C7 — Performance reclamation: caching state across calls

### 9.1 Decision (W5 amended)
(Unchanged.)

### 9.2-9.4 (Unchanged.)

### 9.5 The `apply` hook on the `System` trait (W8 amended + O3' clarification)

```rust
// File: crates/boyko_ecs/src/ecs/core/system/system.rs

pub unsafe trait System: Send + Sync + 'static {
    type Out;
    fn name(&self) -> &'static str;
    fn access(&self) -> &Access;
    fn initialize(&mut self, world: &mut EcsMaster);
    unsafe fn run_unsafe(&mut self, world: UnsafeEcsCell<'_>) -> Self::Out;

    /// Flush deferred mutations recorded by this system's params.
    ///
    /// Called by [`EcsMaster::run_system_once`] after [`run_unsafe`] returns.
    /// Default impl is a no-op; concrete systems delegate to
    /// `<P as SystemParam>::apply(state, meta, world)`.
    ///
    /// # Safety status (O3' clarification)
    ///
    /// **This is a SAFE method** — `fn apply(&mut self, world: &mut EcsMaster)`,
    /// no `unsafe` qualifier. The caller of `apply` (typically
    /// `EcsMaster::run_system_once`) holds `&mut EcsMaster` exclusively; the
    /// borrow checker prevents aliasing at compile time. Identical safety
    /// posture to `SystemParam::apply` (also safe).
    ///
    /// # Why a separate method
    ///
    /// `run_unsafe` runs under `UnsafeEcsCell<'_>` (raw-pointer flavor); the
    /// runner cannot pass `&mut EcsMaster` while the cell is alive. By
    /// splitting into `run_unsafe` → drop-the-cell → `apply(&mut EcsMaster)`,
    /// we get a clean `&mut World` for the flush WITHOUT re-entering the
    /// cell machinery.
    ///
    /// # Safety contract — invariant APP4 (W8)
    ///
    /// **Implementations of `apply` MUST NOT call back into**
    /// **`EcsMaster::run_system_once` or `EcsMaster::run_closure_once`.**
    /// The runtime borrow checker prevents this at compile time
    /// (the caller of `apply` holds `&mut EcsMaster` exclusively; the
    /// inner call would require a second `&mut EcsMaster`, rejected by
    /// Rust's borrow checker). Documented for Phase 9 review: any change
    /// exposing re-entrant system runs from `apply` MUST preserve
    /// borrow-checker prevention.
    #[inline]
    fn apply(&mut self, _world: &mut EcsMaster) {
        // default no-op
    }
}
```

---

## 10. Decision D1 — `CommandQueue` layout (C3 + C4 + C5 FIXES + W1' Send + C2' apply rewrite)

### 10.1 Decision — `CommandQueue` + `RawCommandQueue` split (C3 + C4 FIXES + W1')

```rust
// File: crates/boyko_ecs/src/ecs/core/system/commands/command_queue.rs

use std::mem::{self, MaybeUninit};
use std::ptr::{self, NonNull};

use crate::ecs::core::ecs_master::ecs_master::EcsMaster;

/// Layout constants (C4 FIX — replaces `Packed<T>`).
///
/// Each command in the queue's byte buffer is laid out as:
///
///   offset 0:                       CommandMeta (8 bytes — single fn ptr)
///   offset COMMAND_PAYLOAD_OFFSET:  the command's bytes (sizeof::<C>())
///
/// Both reads and writes use `read_unaligned` / `write_unaligned`.
const COMMAND_PAYLOAD_OFFSET: usize = mem::size_of::<CommandMeta>();

/// Single-fn-ptr dispatch entry. 8 bytes on x86_64.
#[repr(C)]
#[derive(Clone, Copy)]
struct CommandMeta {
    consume_and_drop: unsafe fn(
        value: *mut MaybeUninit<u8>,
        world: Option<NonNull<EcsMaster>>,
        cursor: &mut usize,
    ),
}

/// Type-erased, packed, byte-arena command queue.
///
/// # Layout (CQ1, CQ2, Bevy PR #6391 + #4863)
///
/// * `bytes: Vec<MaybeUninit<u8>>` — packed commands. `MaybeUninit<u8>`
///   permits commands with internal padding (Bevy PR #6391).
/// * `cursor: usize` — apply-time read position. `bytes.len()` at rest;
///   advances during apply. The `RawCommandQueue` split (C3 FIX) sees this
///   field through NonNull so the apply loop never aliases the safe `&mut`.
/// * `panic_recovery: Vec<MaybeUninit<u8>>` — buffer holding the un-run
///   tail of a panicked nested apply. OPAQUE between applies (C2' FIX —
///   Round 2's "Step 0.5" prepend was wrong). Recovery is re-absorbed
///   into `bytes` ONLY inside the catch_unwind Err branch when
///   `start == 0` (top-level), matching Bevy's exact semantics per
///   `command_queue.rs:apply_or_drop_queued`.
///
/// # Size (O2)
///
/// 2 × `Vec` headers (24 B each on 64-bit) + 1 × `usize` cursor = **56 bytes**.
/// Stack-resident per system; 1 cache line.
///
/// # Send / Sync (CQ-SEND1 / W1')
///
/// `CommandQueue: Send` via explicit `unsafe impl` — all commands stored
/// in the byte arena satisfy `Command: Send + 'static` (CQ7 + §11.1.4),
/// so the queue's bytes are transitively Send. `Sync` is NOT implemented
/// — `&CommandQueue` does not allow concurrent push/apply; the per-system
/// ownership (CQ5) gives single-writer access in 8d.
pub(crate) struct CommandQueue {
    pub(crate) bytes: Vec<MaybeUninit<u8>>,
    pub(crate) cursor: usize,
    pub(crate) panic_recovery: Vec<MaybeUninit<u8>>,
}

// SAFETY (CQ-SEND1, W1'):
//   All commands enqueued via `push<C>` satisfy `C: Command: Send + 'static`
//   (the bound is on the push entry point). The byte arena therefore holds
//   only Send bytes. Mirrors Bevy's `unsafe impl Send for CommandQueue`.
unsafe impl Send for CommandQueue {}
// Intentionally NOT Sync. `&CommandQueue` does not permit safe concurrent
// access; Phase 8d gives single-writer ownership via per-system queues
// (CQ5). Phase 9 scheduler is the next layer of arbitration.

/// Raw-pointer twin of [`CommandQueue`]. Apply-time machinery uses this
/// shape to avoid Tree Borrows UB (C3 FIX).
///
/// # Send / Sync
///
/// `RawCommandQueue: !Send + !Sync` (contains raw `NonNull` pointers; the
/// auto-traits are NOT derived for raw pointers). This is INTENTIONAL —
/// `RawCommandQueue` is a transient stack value inside one `apply` call;
/// it must never cross threads. Phase 9's scheduler arbitrates at the
/// `CommandQueue` (safe wrapper) level, not the raw twin.
#[derive(Clone, Copy)]
struct RawCommandQueue {
    bytes: NonNull<Vec<MaybeUninit<u8>>>,
    cursor: NonNull<usize>,
    panic_recovery: NonNull<Vec<MaybeUninit<u8>>>,
}
```

### 10.2 `CommandQueue::push` — type-erased enqueue (C4 FIX, drops `Packed<T>`)

```rust
impl CommandQueue {
    pub(crate) const fn new() -> Self {
        Self {
            bytes: Vec::new(),
            cursor: 0,
            panic_recovery: Vec::new(),
        }
    }

    /// Pushes a command `cmd: C` into the queue.
    ///
    /// # Layout written
    ///
    /// ```text
    /// [CommandMeta (8 B)][command bytes (sizeof::<C>())]
    /// ```
    ///
    /// Both segments written via `write_unaligned` into successive byte
    /// offsets — no `Packed<T>` wrapper, no reference creation into
    /// unaligned memory.
    ///
    /// # Cost (D1 target: ≤ 20 ns)
    ///
    /// * `bytes.reserve(meta + cmd size)` — amortised, cold on first growth.
    /// * Two unaligned writes.
    /// * `bytes.set_len(...)`.
    pub(crate) fn push<C: Command>(&mut self, cmd: C) {
        let meta = CommandMeta {
            consume_and_drop: consume_and_drop_glue::<C>,
        };
        let cmd_size = mem::size_of::<C>();
        let total = COMMAND_PAYLOAD_OFFSET + cmd_size;
        let old_len = self.bytes.len();
        self.bytes.reserve(total);
        // SAFETY (CQ1, CQ2, CQ-PACK1):
        //   - `bytes.capacity() >= old_len + total` post-reserve.
        //   - MaybeUninit<u8> is byte-pattern-agnostic — writes to uninit
        //     slots are sound.
        //   - Both writes are `write_unaligned`; the byte layout requires
        //     no alignment. We never construct `&` or `&mut` references
        //     into the byte slot — CQ-PACK1.
        //   - `set_len` reflects the bytes we just initialised; reads via
        //     `read_unaligned` are sound (CQ2).
        unsafe {
            let base = self.bytes.as_mut_ptr().add(old_len);
            ptr::write_unaligned(base as *mut CommandMeta, meta);
            ptr::write_unaligned(
                base.add(COMMAND_PAYLOAD_OFFSET) as *mut C,
                cmd,
            );
            self.bytes.set_len(old_len + total);
        }
        // `cmd` was moved into `write_unaligned`'s argument slot then
        // bitwise-copied into the queue's bytes. The queue's bytes are the
        // new logical owner. No local `Drop` runs (write_unaligned is a
        // bitwise copy that does NOT invoke Drop on the destination — and
        // the destination was MaybeUninit anyway).
    }

    /// Constructs a [`RawCommandQueue`] borrowed from `self`. The safe
    /// `&mut self` is released for the duration of the returned struct's
    /// use (the apply loop) by passing the raw struct by-value into the
    /// unsafe function. Tree Borrows OK (C3 FIX).
    fn raw(&mut self) -> RawCommandQueue {
        RawCommandQueue {
            // SAFETY (C3): `&raw mut` mints a pointer from `&mut` without
            // creating an intermediate reference. The pointer is
            // non-null because it's derived from a live `&mut`.
            bytes: unsafe { NonNull::new_unchecked(&raw mut self.bytes) },
            cursor: unsafe { NonNull::new_unchecked(&raw mut self.cursor) },
            panic_recovery: unsafe {
                NonNull::new_unchecked(&raw mut self.panic_recovery)
            },
        }
    }
```

### 10.3 `CommandQueue::apply` — safe wrapper (C3 + C5 FIXES + C2' Step 0.5 DELETED)

```rust
    /// Apply-then-drop all queued commands.
    ///
    /// # Semantics (Bevy-mirror — C2' RESOLUTION, Round 2's Step 0.5 DELETED)
    ///
    /// 1. Early-out if `bytes` is empty (3 ns target). `panic_recovery` is
    ///    OPAQUE — its emptiness does NOT affect early-out decision; recovery
    ///    only matters inside the catch_unwind Err branch.
    /// 2. Snapshot `start = cursor` (always 0 in current single-level use).
    /// 3. Set `cursor = bytes.len()` BEFORE the walk (prevents re-entrancy
    ///    confusion if a command-during-apply pushes more commands; those
    ///    new commands extend bytes past the snapshotted stop).
    /// 4. Walk commands from `start` to the snapshotted `stop`.
    /// 5. Per command:
    ///    a. Read meta.
    ///    b. Advance `local_cursor` PAST the meta header AND past the
    ///       command's bytes (W3' FIX — cursor advance is BEFORE
    ///       consume_and_drop, so the panicked command is NOT in the
    ///       recovery range).
    ///    c. Call `consume_and_drop` under `catch_unwind`.
    ///    d. On panic: capture `bytes[local_cursor..bytes.len()]` into
    ///       `panic_recovery` (excludes the panicker since cursor was
    ///       already advanced). Shrink `bytes` to `start`. If
    ///       `start == 0` (top-level), `bytes.append(panic_recovery)` to
    ///       re-drive the recovery on next apply. Propagate via
    ///       `resume_unwind`.
    /// 6. On success: shrink `bytes` to `start`. `cursor = start`.
    ///    `panic_recovery` is UNTOUCHED.
    ///
    /// # Why panic_recovery is OPAQUE between calls (C2' FIX)
    ///
    /// Round 2's "Step 0.5" prepended `panic_recovery` to `bytes` at the
    /// start of every `apply`. This was WRONG — it changed the semantic
    /// from "Bevy mirror" to "architect-invented".
    ///
    /// Bevy's actual flow:
    /// - Top-level apply panics. Err branch: `bytes.set_len(0)`, then
    ///   `bytes.append(panic_recovery)`. Now `bytes` contains the
    ///   redriven recovery. `resume_unwind`.
    /// - Caller catches panic, calls apply again. `bytes` is the redriven
    ///   set; `panic_recovery` is empty. Walk proceeds.
    ///
    /// In Phase 8d's single-level model, `start` is always 0; the
    /// `start == 0` branch always fires; recovery is always re-absorbed
    /// into `bytes` on the same panicking call. The next apply walks
    /// `bytes` (no special-casing).
    ///
    /// # Nested apply (Phase 9 forward-compat)
    ///
    /// If a future Phase 9 design has a command call `world.flush_commands()`
    /// during its own apply, `start > 0` is possible. In that case, on
    /// panic, recovery is kept SEPARATE from `bytes` (so the outer apply
    /// can see it). The outer apply's catch_unwind Err branch with
    /// `start == 0` would then re-absorb. Phase 8d does NOT exercise this
    /// path; APP4 forbids re-entry.
    pub(crate) fn apply(&mut self, world: &mut EcsMaster) {
        // Empty-queue early-out (D1 target: ≤ 3 ns). panic_recovery is
        // OPAQUE — if it's non-empty without bytes being non-empty,
        // something violated Bevy's invariant (should be impossible since
        // recovery only fills on panic and Bevy re-absorbs to bytes in
        // the same call). debug_assert.
        debug_assert!(
            self.panic_recovery.is_empty() || !self.bytes.is_empty(),
            "panic_recovery non-empty implies bytes non-empty (Bevy invariant)"
        );
        if self.bytes.is_empty() {
            return;
        }

        // Construct the raw twin; release the safe `&mut self` for the
        // duration of the walk (C3 FIX).
        let mut raw = self.raw();
        // SAFETY (C3, CQ4):
        //   - `raw` is derived from `&mut self` via `&raw mut`. No
        //     intermediate `&` / `&mut` references are created — Tree
        //     Borrows satisfied.
        //   - For the duration of `apply_or_drop_queued`, we do NOT touch
        //     `self`'s fields directly; `raw` is the sole accessor.
        //   - `world` is passed as `NonNull<EcsMaster>` (mints from
        //     `&mut EcsMaster` via `NonNull::from`). The world borrow is
        //     not aliased — we hold `&mut EcsMaster` exclusively.
        unsafe {
            raw.apply_or_drop_queued(Some(NonNull::from(&mut *world)));
        }
    }
}

impl RawCommandQueue {
    /// Walk the queue; apply or drop each command in turn.
    ///
    /// # SAFETY (C3, CQ2, CQ4)
    ///
    /// * The caller (always `CommandQueue::apply` or `Drop`) holds
    ///   exclusive access to the underlying `CommandQueue` for the
    ///   duration of this call.
    /// * The `world` NonNull, if `Some`, points at a live `EcsMaster`
    ///   that the caller has exclusive `&mut` access to.
    /// * Reentry from inside a command's `apply` body is forbidden by
    ///   APP4 (the command would have to call `run_system_once`, which
    ///   the borrow checker rejects).
    unsafe fn apply_or_drop_queued(&mut self, world: Option<NonNull<EcsMaster>>) {
        // SAFETY: NonNull → &mut for the duration of the call.
        let start = unsafe { *self.cursor.as_ref() };
        let stop = unsafe { self.bytes.as_ref().len() };
        let mut local_cursor = start;
        // Prevent commands run during this walk from re-entering the
        // outer cursor's range. Bevy semantic.
        unsafe { *self.cursor.as_mut() = stop; }

        while local_cursor < stop {
            // Read meta at local_cursor.
            // SAFETY (CQ2): bytes were populated by `push<C>`, which wrote
            //   the meta at this offset via write_unaligned.
            let meta = unsafe {
                self.bytes
                    .as_mut()
                    .as_mut_ptr()
                    .add(local_cursor)
                    .cast::<CommandMeta>()
                    .read_unaligned()
            };

            // Advance past the meta header.
            local_cursor += COMMAND_PAYLOAD_OFFSET;

            // Pointer to the command's bytes.
            let cmd_ptr = unsafe {
                self.bytes.as_mut().as_mut_ptr().add(local_cursor)
            };

            // W3' FIX: cursor advancement for the command's payload happens
            // INSIDE `consume_and_drop_glue` (it knows sizeof::<C>()). After
            // the glue returns OR panics, `local_cursor` has been advanced
            // past the command's bytes (the panicker's `cursor += size_of::<C>()`
            // runs UNCONDITIONALLY BEFORE its `cmd.apply(world)` — see §10.4).
            #[cfg(feature = "std")]
            {
                use std::panic::AssertUnwindSafe;
                let f = AssertUnwindSafe(|| unsafe {
                    (meta.consume_and_drop)(cmd_ptr, world, &mut local_cursor);
                });
                let result = std::panic::catch_unwind(f);

                if let Err(payload) = result {
                    // C5 + W3' FIX (Bevy-mirror):
                    //   - `local_cursor` was already advanced past the
                    //     panicker's bytes by consume_and_drop_glue (W3').
                    //   - Copy [local_cursor..bytes.len()] to recovery —
                    //     this captures the commands AFTER the panicker
                    //     (the panicker is SKIPPED on redrive — W3').
                    //   - Use `current_stop = bytes.len()` (not the
                    //     snapshotted `stop`) to pick up any commands the
                    //     panicker may have enqueued before panicking.
                    //   - Shrink bytes to `start`; reset cursor.
                    //   - If `start == 0` (top-level), append recovery
                    //     back to bytes for next-apply retry.
                    let panic_recovery = unsafe { self.panic_recovery.as_mut() };
                    let bytes = unsafe { self.bytes.as_mut() };
                    let current_stop = bytes.len();
                    // Convert MaybeUninit<u8> slice — extend_from_slice
                    // works because both buffers are Vec<MaybeUninit<u8>>.
                    panic_recovery.extend_from_slice(&bytes[local_cursor..current_stop]);
                    // SAFETY: shrinking the Vec; bytes 0..start are valid.
                    unsafe { bytes.set_len(start); }
                    unsafe { *self.cursor.as_mut() = start; }
                    if start == 0 {
                        bytes.append(panic_recovery);
                        // After this, panic_recovery is empty (append moves).
                        // bytes now contains the redriven commands; next
                        // apply will walk them.
                    }
                    std::panic::resume_unwind(payload);
                }
            }
            #[cfg(not(feature = "std"))]
            unsafe {
                (meta.consume_and_drop)(cmd_ptr, world, &mut local_cursor);
            }
        }

        // Success path: shrink to `start`.
        // SAFETY: 0..start was valid before; the applied range is no
        //   longer accessible via the queue's logical view.
        unsafe {
            self.bytes.as_mut().set_len(start);
            *self.cursor.as_mut() = start;
        }
    }
}
```

### 10.4 `consume_and_drop_glue::<C>` — per-type fnptr body (CQ-PACK1 + W3' cursor advance)

```rust
/// SAFETY contract on the fnptr (CQ2 + CQ-PACK1):
///   - `value` points at `sizeof::<C>()` bytes that form a valid `C`
///     (written by `push<C>` via `write_unaligned`).
///   - The caller (apply_or_drop_queued) holds exclusive access to those
///     bytes.
///   - `world`, if `Some`, points at a live `EcsMaster` the caller holds
///     exclusively.
///   - CQ-PACK1: we use `read_unaligned` on `value as *mut C`. We NEVER
///     construct `&C` or `&mut C` into the unaligned slot.
///
/// # W3' cursor advancement
///
/// The cursor is advanced UNCONDITIONALLY past the command's bytes
/// BEFORE `cmd.apply(world)` runs. This ensures that if `apply` panics,
/// the apply loop's catch_unwind Err branch sees `local_cursor` already
/// past the panicked command — the recovery range
/// `bytes[local_cursor..bytes.len()]` excludes the panicker. The panicker
/// is therefore SKIPPED on the next apply's redrive (Bevy semantic).
///
/// # W1'' Drop-during-unwind / single-drop guarantee
///
/// If `cmd.apply(world)` panics, the local `cmd: C` is already MOVED out
/// of `value` by `ptr::read_unaligned` on entry. When the panic begins
/// unwinding, Rust's drop-elaboration drops `cmd` exactly once (the local
/// going out of scope on the unwind path). The byte slot at `value` is
/// left logically uninitialized — its content was moved into the local,
/// and the byte storage is not re-read by anyone (the caller's cursor
/// already advanced past it via W3'). The outer `apply`'s
/// `catch_unwind` Err branch then handles bytes `[local_cursor..len]`,
/// which by W3' excludes the current command. The net effect:
///
///   - No double-drop: `cmd` drops once on the unwind path; the byte
///     slot it came from is never re-processed.
///   - No leak on panic: the panicker drops cleanly via local unwind;
///     survivors `[local_cursor..len]` drop via the recovery redrive.
unsafe fn consume_and_drop_glue<C: Command>(
    value: *mut MaybeUninit<u8>,
    world: Option<NonNull<EcsMaster>>,
    cursor: &mut usize,
) {
    // SAFETY (CQ-PACK1): read_unaligned does not require alignment and
    //   does not create an intermediate reference.
    let cmd: C = unsafe { ptr::read_unaligned(value as *mut C) };

    // W3' FIX: advance cursor PAST this command's bytes BEFORE running
    // its apply. The meta header was already advanced by the caller; we
    // own the payload-size advance. If `cmd.apply` panics below, cursor
    // is already past us — recovery excludes us.
    *cursor += mem::size_of::<C>();

    if let Some(world_ptr) = world {
        // SAFETY (CQ2): caller holds &mut EcsMaster exclusively; the
        //   NonNull is alive for the call duration.
        let world: &mut EcsMaster = unsafe { &mut *world_ptr.as_ptr() };
        cmd.apply(world);
    } else {
        // Drop-only path: cmd goes out of scope; Drop runs.
        drop(cmd);
    }
}
```

### 10.5 `CommandQueue::Drop` — drop-glue (unchanged from Round 2)

```rust
impl Drop for CommandQueue {
    fn drop(&mut self) {
        // Drain bytes (un-applied commands going out of scope).
        // Drop-only path: world = None.
        if !self.bytes.is_empty() {
            let mut raw = self.raw();
            // SAFETY (CQ4): drop-only path; we hold exclusive &mut self.
            unsafe {
                raw.apply_or_drop_queued(None);
            }
        }
        // Drain panic_recovery similarly (if any leftover from a nested
        // apply panic that didn't reach the start == 0 absorb step).
        if !self.panic_recovery.is_empty() {
            let mut recovery = mem::take(&mut self.panic_recovery);
            self.bytes.append(&mut recovery);
            let mut raw = self.raw();
            // SAFETY: same as above.
            unsafe {
                raw.apply_or_drop_queued(None);
            }
        }
    }
}
```

### 10.6 Alternatives rejected
(Unchanged from Round 2, plus:)

* **(i) Keep "Step 0.5" prepend at start of apply (Round 2).** Rejected — DIVERGED FROM BEVY. C2' FIX deleted it. The Err branch's top-level (`start == 0`) absorb step alone is sufficient; recovery is OPAQUE to apply between calls.
* **(j) Retry the panicked command on redrive instead of skipping (W3').** Rejected — Bevy SKIPS. Retrying would loop on a deterministically-panicking command. Skip means the panic propagates once and the remaining queue runs cleanly on the next apply.

### 10.6.5 W3 RESOLUTION — CommandQueue capacity policy
(Unchanged from Round 2.)

### 10.7 Why this is fast
(Unchanged from Round 2.)

---

## 11. Decision D2 — `Command` trait
(Unchanged from Round 2.)

---

## 12. Decision D3 — `Bundle` trait (C1 FIX — callback API + C1' panic safety + C3' cleanup)

### 12.1 Decision — trait shape (C1 + C2(b) FIXES + C1' panic safety)

```rust
// File: crates/boyko_ecs/src/ecs/core/system/commands/bundle.rs

use crate::ecs::core::component::component::Component;
use crate::ecs::identifiers::primitives::ComponentId;

/// A group of components to insert together when spawning an entity.
///
/// # Send + Sync + 'static (B3, C2(b) RESOLUTION)
///
/// Bundles are stored inside `SpawnCommand<B>` in the queue's byte arena;
/// the bounds are required so the containing system is Send+Sync. Bundle
/// inherits Send+Sync transitively from `(C: Component + Send + Sync,)`.
///
/// # The `for_each_component_bytes` callback API (C1 FIX)
///
/// Round 1's Storage-trait design stored `&[u8]` slot pointers and
/// transmuted them to `&'static [u8]` — unsound. The callback API:
/// the bundle invokes the caller's `FnMut` once per component (N times
/// total for an arity-N bundle), passing `(ComponentId, &[u8])`. The
/// slice borrows from the bundle's stack frame for the duration of the
/// callback chain. No transmute; no Storage trait; no per-spawn
/// allocation.
///
/// # Panic safety (B4, C1' RESOLUTION)
///
/// If the callback panics mid-iteration, the unfinished components LEAK
/// (their `Drop` impls do NOT run; no double-drop with archetype-side
/// ownership). This is achieved by wrapping every destructured tuple
/// element in `ManuallyDrop<T>` BEFORE any callback runs. Trade-off:
/// "leak on user-side panic" is preferable to "double-drop UB".
/// User-side panic in a system body is already an exceptional path;
/// memory leak < memory unsafety.
///
/// # Safety
///
/// Implementations MUST uphold:
/// * **B1** — `component_ids()` returns a deterministic, canonical order
///   (by `ComponentId.0` ascending). The user-visible tuple order may
///   differ; the impl must sort internally.
/// * **B2** — `for_each_component_bytes(self, f)` invokes `f` in the
///   same canonical order as `component_ids()`.
/// * **B3** — `Bundle: Send + Sync + 'static`. Required for Phase 9.
/// * **B4** — On callback panic mid-iteration, unfinished components
///   are LEAKED (no Drop), not double-dropped. Achieved via
///   ManuallyDrop wrappers around every destructured element.
pub trait Bundle: Send + Sync + 'static {
    /// Returns the canonical-order [`ComponentId`] list. `&'static` slice
    /// via `bundle_slot_for` per-type cache (§12.3).
    fn component_ids() -> &'static [ComponentId];

    /// Invokes `f` once per component in canonical order (B2).
    ///
    /// # Panic safety (B4)
    ///
    /// If `f` panics on iteration `i < N`, components `i..N` leak.
    /// Components `0..i` were already transferred via `f` (their bytes
    /// memcpy'd into the archetype slot); the archetype now owns those.
    /// Drop is suppressed for all elements via ManuallyDrop wrappers.
    ///
    /// # FnMut bound
    ///
    /// `F: FnMut` — the callback is invoked N times (once per component).
    /// `FnMut` is the minimal bound that permits N >= 2 invocations and
    /// allows the caller to mutate captured state (e.g., a slot counter).
    fn for_each_component_bytes<F: FnMut(ComponentId, &[u8])>(self, f: F);
}
```

### 12.2 (REMOVED — Round 1's Storage trait is deleted in Round 2.)

### 12.3 Concrete impls (W7 + C1' panic safety + C3' cleanup — single canonical code block)

```rust
use std::any::TypeId;
use std::collections::HashMap;
use std::mem::{self, ManuallyDrop};
use std::sync::{OnceLock, RwLock};

// === Arity 1 ===

impl<C> Bundle for (C,)
where
    C: Component + Send + Sync,
{
    fn component_ids() -> &'static [ComponentId] {
        bundle_slot_for::<Self>(|| {
            let arr = [C::component_id()];
            // Arity 1: already sorted.
            Box::leak(Box::new(arr)) as &'static [ComponentId; 1]
        })
    }

    fn for_each_component_bytes<F: FnMut(ComponentId, &[u8])>(self, mut f: F) {
        let (c,) = self;
        // C1' (B4) — ManuallyDrop suppresses C's Drop. The archetype is
        // the new logical owner once `f` memcpy's the bytes. On callback
        // panic, c is LEAKED (no double-drop).
        let c = ManuallyDrop::new(c);
        // SAFETY (CQ-PACK1-analog): we take *const u8 from a live local;
        //   the slice's lifetime is bounded by `c`'s scope (this function).
        //   `f` typically memcpy's the bytes; the slice never escapes.
        let bytes: &[u8] = unsafe {
            std::slice::from_raw_parts(
                (&raw const *c) as *const u8,
                mem::size_of::<C>(),
            )
        };
        f(C::component_id(), bytes);
        // Falls out of scope; ManuallyDrop suppresses Drop. If f panicked
        // above, ManuallyDrop still suppresses (unwind path).
    }
}

// === Arity 2 ===

impl<A, B> Bundle for (A, B)
where
    A: Component + Send + Sync,
    B: Component + Send + Sync,
{
    fn component_ids() -> &'static [ComponentId] {
        bundle_slot_for::<Self>(|| {
            let mut arr = [A::component_id(), B::component_id()];
            arr.sort_by_key(|id| id.0);
            Box::leak(Box::new(arr)) as &'static [ComponentId; 2]
        })
    }

    fn for_each_component_bytes<F: FnMut(ComponentId, &[u8])>(self, mut f: F) {
        let (a, b) = self;
        // C1' (B4) — wrap both in ManuallyDrop UPFRONT. Now even if
        // `f(a_id, ...)` succeeds and `f(b_id, ...)` panics, neither
        // A::Drop nor B::Drop runs on the stack-locals — no double-drop
        // with the archetype that now owns a's bytes.
        let a = ManuallyDrop::new(a);
        let b = ManuallyDrop::new(b);

        let a_id = A::component_id();
        let b_id = B::component_id();
        let a_bytes: &[u8] = unsafe {
            std::slice::from_raw_parts(
                (&raw const *a) as *const u8,
                mem::size_of::<A>(),
            )
        };
        let b_bytes: &[u8] = unsafe {
            std::slice::from_raw_parts(
                (&raw const *b) as *const u8,
                mem::size_of::<B>(),
            )
        };
        // Emit in canonical order — B1.
        if a_id.0 <= b_id.0 {
            f(a_id, a_bytes);
            f(b_id, b_bytes);
        } else {
            f(b_id, b_bytes);
            f(a_id, a_bytes);
        }
        // ManuallyDrop suppresses Drop on a and b unconditionally.
    }
}

// === Arity 3 / Arity 4 ===
//
// Mechanical extension of the arity-2 pattern:
//   1. Destructure tuple.
//   2. Wrap EACH element in ManuallyDrop UPFRONT (before any f call).
//   3. Compute &[u8] slices.
//   4. Sort by ComponentId; emit in canonical order.
//
// For arity 4, sort the 4-element `[(ComponentId, &[u8])]` array on the
// stack (no allocation; arity is const). This is fine — sort cost is
// negligible vs the per-component memcpy.

// === The per-Bundle-type ID-slice cache (W7 + O1') ===

/// Per-Bundle-type cache for the sorted ComponentId slice.
///
/// # Implementation
///
/// `OnceLock<RwLock<HashMap<TypeId, &'static [ComponentId]>>>` —
/// module-level shared map keyed by `TypeId::of::<B>()`. First call
/// per Bundle type: leak a `Box<[ComponentId; N]>` and cache the
/// `&'static` pointer.
///
/// # Memory cost (O1' DOCUMENTED)
///
/// Each distinct Bundle type leaks one `[ComponentId; N]` (typically
/// 4–16 bytes for N ∈ 1..=4). Bounded by `N_BUNDLE_TYPES × 16 B`. A
/// project with ≤ 100 distinct bundle types leaks ≤ 1.6 KB — negligible.
/// Phase 9 §9.10 will migrate to const-fn-in-traits when stable
/// (eliminates the leak entirely).
///
/// # Cost per call
///
/// Cached (hot): ~30 ns — RwLock read + HashMap::get + Acquire-load.
/// Uncached (cold, once per Bundle type): ~100 ns — write-lock + Box +
/// HashMap::insert.
fn bundle_slot_for<B: 'static>(
    init: impl FnOnce() -> &'static [ComponentId],
) -> &'static [ComponentId] {
    static CACHE: OnceLock<RwLock<HashMap<TypeId, &'static [ComponentId]>>> =
        OnceLock::new();
    let cache = CACHE.get_or_init(|| RwLock::new(HashMap::new()));

    let tid = TypeId::of::<B>();
    if let Some(ids) = cache.read().expect("cache poisoned").get(&tid).copied() {
        return ids;
    }

    let mut w = cache.write().expect("cache poisoned");
    if let Some(ids) = w.get(&tid).copied() {
        return ids;
    }
    let ids = init();
    w.insert(tid, ids);
    ids
}
```

**Trade-off (W7 honest cost):** ~30 ns per cached `component_ids()` call. Cold path (called once per SpawnCommand::apply, not per-frame enqueue). Acceptable.

**Arity 3 and 4:** mechanical extension; sort the 3 or 4 `(ComponentId, &[u8])` pairs on stack and emit in canonical order. ManuallyDrop wraps every element upfront (B4 invariant).

### 12.4 `SpawnCommand<B>::apply` — uses callback + stack collector (W4' FIX — drop ArchetypeRow)

```rust
impl<B: Bundle> Command for SpawnCommand<B> {
    fn apply(self, world: &mut EcsMaster) {
        let archetype_id = world.get_or_create_archetype(B::component_ids());

        // W4' FIX — use a stack-allocated collector + the EXISTING
        // EcsMaster::create_entity(archetype_id, &[(ComponentId, &[u8])]).
        // No new EcsMaster API (no ArchetypeRow, no create_entity_with).
        //
        // KEY LIFETIME OBSERVATION:
        //   The bundle's destructured locals (wrapped in ManuallyDrop) live
        //   for the duration of `for_each_component_bytes`'s function body.
        //   The callback's `&[u8]` slices borrow from those locals. The
        //   slices stored in `slots[..]` ARE valid as long as we are
        //   INSIDE `for_each_component_bytes`'s call. The trick: invoke
        //   `world.create_entity(...)` from inside the callback chain,
        //   not after `for_each_component_bytes` returns.
        //
        // The callback chain: for_each_component_bytes invokes our
        // FnMut N times (once per component). On the N-th call (last
        // component, count == arity - 1), we have all slices collected
        // and call create_entity. The bundle's locals are still alive
        // (we are still inside the function); the slices are valid.
        //
        // To make this work with FnMut: track count in a `let mut` outside
        // the closure; the closure captures `&mut count` and `&mut slots`.
        // On the last invocation, perform the create_entity call inline.

        let arity = B::component_ids().len();
        debug_assert!(arity > 0 && arity <= 4, "Phase 8d arity ceiling: 1..=4");

        // Stack-allocated slot array. Use MaybeUninit to avoid the
        // (ComponentId(0), &[]) zero-initialization cost (a no-op the
        // compiler probably optimizes, but explicit).
        let mut slots: [mem::MaybeUninit<(ComponentId, &[u8])>; 4] = [
            mem::MaybeUninit::uninit(),
            mem::MaybeUninit::uninit(),
            mem::MaybeUninit::uninit(),
            mem::MaybeUninit::uninit(),
        ];
        let mut count = 0usize;

        self.bundle.for_each_component_bytes(|id, bytes| {
            // SAFETY (B2): bundle invokes us at most arity times.
            debug_assert!(count < 4, "Bundle arity > 4 in Phase 8d");
            slots[count].write((id, bytes));
            count += 1;

            // On the LAST invocation, call create_entity while the bundle's
            // locals (and thus the byte slices) are still alive.
            if count == arity {
                let initialized: &[(ComponentId, &[u8])] = unsafe {
                    // SAFETY (W2''):
                    //   - `slots[0..count]` were each initialized by
                    //     `.write()` above (`count` was incremented
                    //     immediately after each write, before this
                    //     point).
                    //   - `MaybeUninit<T>` and `T` have identical layout
                    //     (size, align, repr) per the Rust reference. The
                    //     cast `*const MaybeUninit<T>` → `*const T` is
                    //     therefore layout-compatible.
                    //   - We construct a slice of exactly `count`
                    //     elements (not 4), so uninitialized slots
                    //     `[count..4]` are NEVER accessed.
                    //   - The resulting `&[(ComponentId, &[u8])]` borrows
                    //     from `slots` for the duration of this block.
                    //     The inner `&[u8]` lifetimes borrow from the
                    //     bundle's stack frame (the FnMut callback chain
                    //     keeps the bundle's ManuallyDrop locals alive
                    //     until `for_each_component_bytes` returns).
                    std::slice::from_raw_parts(
                        slots.as_ptr() as *const (ComponentId, &[u8]),
                        count,
                    )
                };
                // W3'' FIX: the returned Entity is discarded —
                // SpawnCommand does not surface it back to the caller in
                // Phase 8d. Phase 11 will add a
                // `SpawnCommandReturning<B>` variant that pre-allocates
                // an Entity and surfaces it pre-apply (Bevy-style
                // `commands.spawn(...).id()`).
                let _ = world
                    .create_entity(archetype_id, initialized)
                    .expect("create_entity failed inside SpawnCommand::apply");
            }
        });

        // After for_each_component_bytes returns:
        //   - ManuallyDrop suppressed each component's Drop.
        //   - The bytes were memcpy'd into the archetype by create_entity.
        //   - The archetype owns the components now.
        // If arity was 0 (impossible per debug_assert + Phase 8d ceiling),
        // create_entity is never called — this would be a logic bug.
        debug_assert_eq!(
            count, arity,
            "Bundle invoked for_each_component_bytes {} times, expected {}",
            count, arity,
        );
    }
}
```

**Lifetime soundness re-trace (W4' final):**

1. `self.bundle: B` is moved into `for_each_component_bytes` (by-value `self`).
2. Inside `for_each_component_bytes`, components are destructured into stack locals wrapped in `ManuallyDrop` (B4).
3. The callback receives `&[u8]` slices borrowing from those `ManuallyDrop<T>` locals. The locals live for the function's full duration.
4. On the LAST callback invocation (count == arity), `world.create_entity(archetype_id, &slots[..count])` is called from INSIDE the callback — the bundle's locals are still alive; the slices are valid.
5. `create_entity` memcpy's each `&[u8]` into its archetype slot (via `copy_nonoverlapping`). The archetype is the new owner.
6. `for_each_component_bytes` returns; ManuallyDrop suppresses Drop on all locals (their storage is now in the archetype; double-drop avoided).

This pattern requires NO new EcsMaster API. `ArchetypeRow` is dropped entirely.

### 12.5 Alternatives rejected
(Round 1 §12.4 plus:)

* **(f) Per-bundle stack-allocated array with `&'static [u8]` transmute (Round 1).** Rejected — UNSOUND. (C1 FIX.)
* **(g) Allocate `Vec<(ComponentId, &[u8])>` inside `for_each_component_bytes`.** Rejected — per-spawn allocation forbidden.
* **(h) Bundle returns a `[u8; SUM_SIZE]` const-sized blob.** Rejected — requires const-generic arithmetic on component sizes, unstable.
* **(i) Add `EcsMaster::create_entity_with(archetype_id, count, fill: FnOnce(&mut ArchetypeRow))` (Round 2).** Rejected (W4') — `ArchetypeRow` was never defined; designing it would add a new API surface for no benefit. The stack-collector pattern above achieves the same effect using existing `create_entity` and adds nothing to EcsMaster's public surface.
* **(j) Skip ManuallyDrop and use `mem::forget` at the end of `for_each_component_bytes` (Round 2).** Rejected (C1') — `mem::forget` at the END is bypassed by stack-unwinding if `f` panics mid-iteration, leading to double-drop UB. ManuallyDrop UPFRONT is panic-safe.

### 12.6 Trade-off
Bundle is **arity 1..=4** in 8d. Callback API + stack collector + existing `create_entity`. No transmute, no Storage trait, no per-spawn alloc, no new EcsMaster API. C1 + C1' + W4' resolved.

### 12.7 Why this is fast
- `for_each_component_bytes(self, f)`: 1 destructure + N ManuallyDrop wraps + N callback invocations. ~5 ns + per-component memcpy cost.
- `create_entity`: ~100-150 ns per Phase 7 entity creation.
- Total: ~120-180 ns per spawn at flush time. Within the 200 ns target.

---

## 13. Decision D4 — `Commands<'s>` SystemParam shape
(Unchanged from Round 2.)

```rust
impl<'s> Commands<'s> {
    pub fn spawn<B: Bundle>(&mut self, bundle: B) { self.queue.push(SpawnCommand { bundle }); }
    pub fn despawn(&mut self, entity: Entity) { self.queue.push(DespawnCommand { entity }); }
    pub fn add<C: Command>(&mut self, cmd: C) { self.queue.push(cmd); }
}
```

---

## 14. Decision D5 — Spawn/Despawn entity id deferral (CQ6)
## 15. Decision D6 — `EntityCommands` chaining (DEFERRED Phase 9)
## 16. Decision D7 — `apply` flush semantics
## 17. Decision D8 — Integration with `run_system_once`
(All four unchanged from Round 2.)

---

## 18. SAFETY invariants — Phase 8c+8d full list (revised — C1', C2', W1', O3' updates)

| ID | Invariant |
|----|-----------|
| **FS1** | FunctionSystem::initialize idempotent |
| **FS2** | State + meta cached across run_unsafe / apply |
| **FS3** | Double-FnMut HRTB bound load-bearing for closure inference (C6-verified; W2' canonical user form `\|q: Query<&A>\|` validated by §4.7 elided-form test) |
| **IS1** | Marker uniqueness per arity |
| **IS2** | IntoSystem identity-vs-function blanket disjoint via marker |
| **CQ1** | bytes: Vec<MaybeUninit<u8>> (Bevy PR #6391) |
| **CQ2** | CommandMeta single-fnptr dispatch (8 B) |
| **CQ3** | read_unaligned / write_unaligned everywhere (no Packed<T>) |
| **CQ-PACK1** | Never construct `&` or `&mut` references into the queue's byte slot |
| **CQ4** | Panic-recovery via `apply_or_drop_queued`: panicker SKIPPED (cursor advanced before consume_and_drop returns; W3'); recovery captures bytes AFTER panicker; top-level (start == 0) re-appends recovery to bytes for next-apply retry (C5 FIX — Bevy mirror) |
| **CQ-SEND1** | **NEW (W1')** `unsafe impl Send for CommandQueue` — all commands satisfy `Command: Send + 'static` ⇒ bytes are transitively Send. `!Sync` (single-writer). `RawCommandQueue: !Send + !Sync` (raw pointers; intentional). |
| **CQ5** | Per-system queue ownership; Phase 9 scheduler serialises apply |
| **CQ6** | Commands::spawn does NOT return Entity in 8d (Phase 9 §9.6) |
| **CQ7** | Command::apply consumes by-value under exclusive &mut EcsMaster |
| **B1** | Bundle::component_ids canonical order (sorted by ComponentId.0 asc) |
| **B2** | for_each_component_bytes callback invokes in canonical order (B1+B2 lock the contract; FnMut for repeated invocation per C3') |
| **B3** | Bundle: Send + Sync + 'static. Inherited from Component bound on impls. |
| **B4** | **NEW (C1')** Bundle::for_each_component_bytes panic safety: on callback panic mid-iteration, unfinished components LEAK (no Drop), not double-drop. Achieved via ManuallyDrop wrappers around every destructured element BEFORE any callback runs. Leak < UB. |
| **APP1** | System::apply called by run_system_once after run_unsafe; UnsafeEcsCell consumed |
| **APP1'** | **NEW (O3')** Both `System::apply` and `SystemParam::apply` are SAFE methods — `fn apply(&mut self, world: &mut EcsMaster)`, no `unsafe` qualifier. Caller (always `run_system_once` directly or the System::apply default delegation) holds `&mut EcsMaster` exclusively; no aliasing risk; no UnsafeEcsCell involved. |
| **APP2** | Tuple SystemParam::apply calls each in declaration order |
| **APP3** | Tuple apply ordering REQUIRED, not best-effort. `(Commands, OtherCommands)` MUST flush Commands first. Tested in Step 10. |
| **APP4** | System::apply MUST NOT re-enter EcsMaster::run_system_once / run_closure_once. Borrow checker enforces. |

---

## 19. Data structures — size verification (O2 FIX + W1' Send notes)

| Type | Size | Cache lines | Send / Sync |
|------|------|-------------|-------------|
| `IntoSystem` trait | ZST | n/a | n/a |
| `SystemParamFunction<Marker>` trait | ZST | n/a | n/a |
| `FunctionSystem<F, Marker>` | `sizeof(F) + 8 + sizeof(Param::State) + 224 (SystemMeta)` ≈ ~232 + state + captures | 4-5 lines | Send + Sync via System trait bound |
| `Marker` (e.g. `fn(Query<&A>, Res<B>)`) | 0 | n/a | n/a |
| `CommandMeta` | 8 B | < 1 line | Copy |
| **`CommandQueue`** | **56 B** (2 × Vec header + cursor) | 1 line | **Send (CQ-SEND1)**, `!Sync` |
| `RawCommandQueue` | 24 B (3 × NonNull) | < 1 line | **`!Send + !Sync`** (raw ptrs) |
| `Commands<'s>` | 8 B (`&'s mut CommandQueue` ptr) | < 1 line | `!Send` via lifetime; `!Sync` |
| `Command` trait | ZST | n/a | `Send + 'static` (required) |
| `SpawnCommand<B>` | `sizeof(B)` | per-bundle | Send via B: Send |
| `DespawnCommand` | 8 B (Entity = `u32 + u32`) | < 1 line | Send (POD) |
| `Bundle` trait | ZST | n/a | `Send + Sync + 'static` (required, B3) |

### 19.1 Layout notes (O2 amended)

* `FunctionSystem` field order: func → state → meta → marker (access-frequency descending).
* `CommandQueue`'s 56 B stack overhead per system × 1000 systems = 56 KB. Negligible.
* `RawCommandQueue` (24 B) is constructed transiently inside `apply`; never long-lived. Stack-only.

### 19.2 No padding for false sharing
(Unchanged.)

---

## 20. Public API surface delta

### 20.1 New public types (O4 RENAME applied)

```rust
// crates/boyko_ecs/src/ecs/core/system/mod.rs (re-exports)
pub use into_system::{IntoSystem, IsFunctionSystem};
pub use system_param_function::{SystemParamFunction, SystemParamItem, MAX_SYSTEM_PARAM_FN_ARITY};
pub use function_system::FunctionSystem;
pub use commands::Commands;
pub use commands::command::Command;
pub use commands::bundle::Bundle;
// pub(crate) — internal storage detail:
//   CommandQueue, CommandMeta, RawCommandQueue, SpawnCommand, DespawnCommand.
```

### 20.2 Modified `EcsMaster` methods (W4' — no new EcsMaster API)

```rust
impl EcsMaster {
    pub fn run_closure_once<F, Out, Marker>(&mut self, body: F) -> Out
    where
        F: IntoSystem<(), Out, Marker>,
        Marker: 'static,
        Out: 'static;

    pub fn run_system_once<S: System>(&mut self, system: &mut S) -> S::Out;
}
```

**(W4' FIX: `create_entity_with` and `ArchetypeRow` DROPPED.)** Phase 7's existing `create_entity(archetype_id, &[(ComponentId, &[u8])])` is sufficient via the stack-collector pattern in §12.4. The Round 2 `ArchetypeRow` opaque handle is REMOVED from the plan entirely.

### 20.3 Removed types
* `FnOnceSystem<P, F, O>` — deleted.
* `BundleStorage`, `BundleStorage1`, `BundleStorageN` — never existed in shipping form.
* **(W4')** `ArchetypeRow<'a>` and `EcsMaster::create_entity_with` — removed from the plan; never reached implementation.

### 20.4 Modified `System` trait
(Unchanged — adds `fn apply` with default no-op + APP4 rustdoc + APP1' safe-fn note.)

### 20.5 Modified `SystemParam` tuple impl macro
(Unchanged — adds `apply` override per arity.)

---

## 21. Algorithms for critical paths

### 21.1 `Commands::spawn` enqueue (≤ 20 ns)
```text
Step 1: Construct `SpawnCommand<B> { bundle }` on stack       (free; moves bundle)
Step 2: bytes.reserve(COMMAND_PAYLOAD_OFFSET + sizeof::<SpawnCommand<B>>())
                                                              (amortised)
Step 3: write_unaligned(base, meta)                           (1 line write)
Step 4: write_unaligned(base + COMMAND_PAYLOAD_OFFSET, cmd)   (sizeof(SpawnCommand<B>) bytes)
Step 5: bytes.set_len(old_len + total)                        (free)
```

### 21.2 `CommandQueue::apply` (C2' — Step 0.5 deleted)
```text
Step 0: early-out if bytes is empty                              (≤ 3 ns)
Step 1: raw = self.raw()                                         (3 × &raw mut)
Step 2: unsafe { raw.apply_or_drop_queued(Some(world)) }         (per-command walk)
        See §10.3 — single loop, panic-guard via catch_unwind.
        On panic: top-level (start == 0) absorbs recovery into
        bytes via the Err branch; resume_unwind.
        On success: bytes.set_len(start); cursor = start.
```

**(C2' FIX: Round 2's "Step 0.5 prepend recovery to bytes" is DELETED. Recovery is OPAQUE between calls; only the catch_unwind Err branch touches it.)**

### 21.3 `FunctionSystem::run_unsafe` (≤ 8 ns for 1× Res param)
(Unchanged.)

### 21.4 `IntoSystem::into_system` (cold)
(Unchanged.)

### 21.5 `Commands::apply` (≤ 3 ns empty)
(Unchanged.)

### 21.6 `Bundle::component_ids` (cold)
```text
First call per Bundle type:
  Step 1: TypeId::of::<B>() lookup in OnceLock<RwLock<HashMap>>  (~30 ns)
  Step 2: If miss: write-lock + insert leaked `Box<[ComponentId; N]>`  (~100 ns once)
Subsequent calls:
  Step 1: TypeId lookup hit + Acquire load  (~30 ns)
```

### 21.7 `SpawnCommand<B>::apply` (cold per spawn) — W4' stack-collector
```text
Step 1: archetype_id = world.get_or_create_archetype(B::component_ids())
        ~10-30 ns (steady), ~100-500 ns first call.
Step 2: arity = B::component_ids().len(); slots = [MaybeUninit; 4]; count = 0
Step 3: bundle.for_each_component_bytes(|id, bytes| {
            slots[count].write((id, bytes)); count += 1;
            if count == arity {
                world.create_entity(archetype_id, &slots[..count]);
            }
        })
        - Inner per-component: write to slot (~2 ns).
        - On terminal callback: create_entity (~100-150 ns per Phase 7).
Total: ~120-180 ns per spawn.
```

---

## 22. Multithreading model
(Unchanged from Round 2.)

---

## 23. Integration with existing modules

### 23.1 Files created (new)
(Same as Round 2 §23.1, minus the deleted Storage variants. `bundle.rs` ~250 lines.)

### 23.2 Files modified (W4' FIX — no new EcsMaster API)

| File | Change |
|------|--------|
| `crates/boyko_ecs/src/ecs/core/system/system.rs` | Add `fn apply` with default no-op + APP4 docs + APP1' safe-fn note. |
| `crates/boyko_ecs/src/ecs/core/system/mod.rs` | Module re-exports (IsFunctionSystem). |
| `crates/boyko_ecs/src/ecs/core/system/params/tuple_impl.rs` | Add `apply` override per arity (APP3). No const-panic changes (W4 verified-then-defer). |
| `crates/boyko_ecs/src/ecs/core/system/params/mod.rs` | `pub mod commands;`. |
| `crates/boyko_ecs/src/ecs/core/system/fn_once_system.rs` | **Deleted.** |
| `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs` | Rewrite `run_closure_once` (W5+W6 docs). Update `run_system_once`. **(W4' — no new method; `create_entity_with` and `ArchetypeRow` REMOVED from plan.)** Remove `FnOnceSystem` import. |
| `crates/boyko_ecs/tests/system_param_smoke.rs` | Drop turbofish on Phase 8a tests. |
| `crates/boyko_ecs/tests/query_dsl_smoke.rs` | Drop turbofish on Phase 8b tests. |
| `crates/boyko_ecs/benches/system_param.rs` | Add Commands enqueue/apply benches + hoisted-vs-rebuilt bench. |
| `crates/boyko_ecs/benches/query_dsl.rs` | Drop turbofish; reverify ≤30 ns hoisted. |
| `crates/boyko_ecs/benches/query_iter.rs` | Drop turbofish. |

### 23.3 Compatibility checks
(Unchanged.)

### 23.4 No change to Phase 8a/8b semantics
(Unchanged.)

### 23.5 The `FnOnceSystem` removal
(Unchanged.)

---

## 24. Implementation plan — 13 steps

### Step 0 — `System::apply` method added (+ APP4 docs + APP1' safe-fn note)
**Files:** `crates/boyko_ecs/src/ecs/core/system/system.rs`
**Acceptance:** `cargo check` clean; NoopSystem inherits default.

### Step 1 — `IntoSystem` trait + identity blanket
**Files:** `crates/boyko_ecs/src/ecs/core/system/into_system.rs`, `mod.rs`
**Acceptance:** Compile-only assert_impl shim for NoopSystem.

### Step 2 — **C6 HRTB reproducer (3 tests; W2' elided form) + `SystemParamFunction` trait + variadic macro for 1..=12 + arity-0**

**Files:**
* `crates/boyko_ecs/tests/hrtb_reproducer.rs` (new) — §4.7 verbatim (THREE tests).
* `crates/boyko_ecs/src/ecs/core/system/system_param_function.rs` (new).

**Acceptance:**
* **HRTB reproducer compiles** (`cargo test --test hrtb_reproducer`). ALL THREE tests must pass:
  * `closure_compiles_without_turbofish` (Test 1)
  * `function_pointer_compiles_without_turbofish` (Test 2)
  * `closure_with_elided_param_type_compiles` (Test 3, W2') — fully-elided form `|p: StubParam|`.

  **If Test 3 fails: HALT and invoke §4.7.1 fallback.** Test 1 alone is insufficient to validate the headline ergonomic claim `|q: Query<&Position>|`.

### Step 3 — Runtime-panic stubs for arity 13..=24 (Phase 8b B1 lesson)
(Unchanged from Round 2.)

### Step 4 — `FunctionSystem<F, Marker>: System`
(Unchanged.)

### Step 5 — Delete `FnOnceSystem` + rewrite `run_closure_once` / `run_system_once`
(Unchanged from Round 2 except W4': no `create_entity_with` added to EcsMaster.)

### Step 6 — `Bundle` trait + arity-1..=4 impls (C1 + C1' + C3' FIXES: callback API, ManuallyDrop upfront)

**Files:**
* `crates/boyko_ecs/src/ecs/core/system/commands/mod.rs`
* `crates/boyko_ecs/src/ecs/core/system/commands/bundle.rs`
* `crates/boyko_ecs/src/ecs/core/system/mod.rs` (registry)

**Acceptance:**
* `bundle_arity_1_component_ids_returns_single_id`
* `bundle_arity_2_component_ids_canonical_order`
* `bundle_for_each_component_bytes_invokes_in_canonical_order` (B2)
* `bundle_for_each_component_bytes_callback_sees_valid_bytes`
* `bundle_arity_3_and_4_sort_correctness`
* `bundle_oncelock_returns_same_slice_on_repeated_calls` (W7)
* **`bundle_for_each_panics_in_callback_no_double_drop`** (C1' / B4) — define a Component with a Drop counter; spawn `(Counter, Counter)`; panic in the second f invocation; verify Counter::Drop ran 0 times on the stack-locals (ManuallyDrop suppressed) AND zero times on archetype-side (Counter not yet written there).

### Step 7 — `Command` trait + `SpawnCommand` + `DespawnCommand` (W4' — no EcsMaster changes)

**Files:**
* `crates/boyko_ecs/src/ecs/core/system/commands/command.rs` (new).

**Acceptance:**
* `spawn_command_apply_creates_entity` — uses Bundle callback + stack collector + existing `world.create_entity(...)`.
* `despawn_command_apply_removes_entity`.
* `command_send_static_bound_compile_check`.

### Step 8 — `CommandQueue` byte-arena + `CommandMeta` + `RawCommandQueue` + panic-recovery (C2' + W3' FIXES)

**Files:**
* `crates/boyko_ecs/src/ecs/core/system/commands/command_queue.rs`

**Acceptance:**
* `command_queue_empty_apply_is_noop`
* `command_queue_push_then_apply_runs_command`
* `command_queue_push_many_then_apply` (100 commands)
* `command_queue_drop_runs_drop_glue_on_unapplied_commands`
* **`command_queue_panic_in_apply_captures_post_panicker_tail`** (C5 + W3') — push cmd1 (ok), cmd2 (panics), cmd3 (ok). Call apply; expect panic. Verify `panic_recovery.is_empty()` AND `bytes` contains exactly cmd3's bytes (panicker cmd2 is SKIPPED; top-level start==0 absorb re-appended cmd3 to bytes from recovery).
* **`command_queue_panic_skips_panicker_runs_rest_on_redrive`** (W3' RESOLUTION) — same setup as above; catch the first panic at test boundary; call apply AGAIN; verify cmd3 runs successfully. cmd2 NEVER runs again (skip semantic).
* **`command_queue_recovery_opaque_between_applies`** (C2' RESOLUTION) — manually populate `panic_recovery` with cmd_X bytes while keeping `bytes` empty. Call apply. Verify cmd_X does NOT run (recovery is OPAQUE; only the catch_unwind Err branch absorbs it). The test exists to lock-down the C2' behavior so a future refactor cannot accidentally re-introduce "Step 0.5 prepend".
* **`command_queue_top_level_recovery_reabsorbed_on_panic`** — explicit Bevy semantic: panic at start==0 ⇒ `bytes.append(panic_recovery)` runs in the Err branch; `bytes` is non-empty post-panic; next apply walks it.
* `command_queue_drop_after_panic_runs_drop_glue_on_recovery`
* `command_queue_padded_command_writes_soundly` (CQ1 + CQ-PACK1)
* **`command_queue_capacity_not_shrunk_after_apply`** (W3) — push many; apply; capacity ≥ peak.

### Step 9 — `Commands<'s>` SystemParam impl
(Unchanged from Round 2.)

### Step 10 — Update tuple impl macro `apply` (W1 / APP3 test)
(Unchanged from Round 2.)

### Step 11 — End-to-end integration + benches (W5 bench split)
(Unchanged from Round 2.)

### Step 12 — Miri test suite (C1' + C2' + W3' + W2'' additions)

**Files:** `crates/boyko_ecs/tests/miri_phase8cd.rs` (`#[cfg(miri)]`)

**Tests:**
* `miri_command_queue_push_then_apply_no_ub`
* `miri_command_queue_padded_command_no_uninit_ub` (CQ1)
* `miri_command_queue_no_packed_reference_creation` (CQ-PACK1)
* `miri_command_queue_raw_command_queue_no_alias_ub` (C3)
* `miri_command_queue_panic_recovery_no_ub` (C5 + C2')
* **`miri_bundle_for_each_panics_no_double_drop`** (C1' / B4 NEW) — Miri-mode test using a Component whose Drop has a runtime-tracking side-effect (e.g. atomic counter or thread-local Vec). Spawn `(Tracker, Tracker)`; arrange for the second f invocation to panic; verify Miri reports NO double-drop AND that the post-iteration state has Tracker's Drop run 0 times (leaked) — not 1 or 2 times.
* `miri_bundle_for_each_component_bytes_callback_lifetime` (C1)
* **`miri_bundle_slice_cast_arity_1_and_4`** (W2'' NEW) — exercises the `*const MaybeUninit<(ComponentId, &[u8])>` → `*const (ComponentId, &[u8])` slice cast in `SpawnCommand::apply` for both arity-1 `(A,)` and arity-4 `(A, B, C, D)` bundles under `cargo +nightly miri test`. Asserts no UB report from Miri's strict provenance + Tree Borrows checks. Validates that uninitialized slots `[count..4]` are never accessed and that the layout-compatibility cast is sound.
* `miri_function_system_initialize_then_run_no_retag_ub`
* `miri_function_system_apply_after_run_unsafe_no_aliasing`
* `miri_run_closure_once_no_turbofish_arity_4_no_ub`

---

## 25. Metrics and validation
(Unchanged section structure from Round 2.)

### 25.4 `debug_assert!` invariants (revised)

In `FunctionSystem::initialize`: gen_before == gen_after (SP4).
In `FunctionSystem::run_unsafe`: `state.is_some()`.
In `CommandQueue::push`: `bytes.capacity() >= old_len + total`.
In `CommandQueue::apply`'s pre-walk: `panic_recovery.is_empty() || !bytes.is_empty()` (Bevy invariant — recovery non-empty implies bytes non-empty at top-level).
In `RawCommandQueue::apply_or_drop_queued`: pre-loop `*cursor.as_ref() <= bytes.as_ref().len()`.
In `consume_and_drop_glue`: `!value.is_null()`.
In `SpawnCommand::apply`: `arity > 0 && arity <= 4` (Phase 8d ceiling) pre-iteration; `debug_assert_eq!(count, arity)` post-callback (W3'' — B2 invariant: bundle invoked callback exactly `arity` times).
In `Bundle::for_each_component_bytes` (per arity): `slots[count].write(...)` only when `count < arity`.

### 25.5 Benches with targets (W5 split + O2' first-call)
| Bench | Target |
|-------|--------|
| `bench_function_system_run_unsafe_empty_hoisted` | ≤ 5 ns |
| `bench_function_system_run_unsafe_res_param_hoisted` | ≤ 8 ns |
| `bench_run_closure_once_reused_vs_phase_8a_baseline` | ≤ 30 ns per call after first |
| `bench_run_closure_once_per_call_rebuilds` | ≈ 960 ns (baseline) |
| `bench_run_closure_once_first_call_cold` (O2' NEW) | ≈ 1.2 µs (init + dispatch + run + apply) |
| `bench_commands_spawn_one_enqueue` | ≤ 20 ns |
| `bench_commands_despawn_enqueue` | ≤ 15 ns |
| `bench_command_queue_apply_100_spawns` | ≤ 20 µs |
| `bench_command_queue_empty_apply` | ≤ 3 ns |
| `bench_function_system_apply_no_commands` | ≤ 3 ns |

---

## 26. Cross-phase dependencies
(Unchanged.)

---

## 27. Risks and mitigations (revised — C1', C2', W4' updates)

| Risk | Severity | Mitigation |
|------|----------|------------|
| GAT inference fails for closure-arg deduction | **Low after §4.7 reproducer (3 tests; W2' elided form)** | Step 2 reproducer is the canary. Test 3 (W2') validates fully-elided form. Fallback §4.7.1. |
| Double-`FnMut` bound trips rustc | Low | Same canary. |
| `CommandQueue::apply` panic-recovery loses commands | **Low after C5 + C2' + W3' FIXES** | Bevy-mirror semantics (Step 0.5 deleted, panicker SKIPPED, recovery OPAQUE between calls). Step 8 acceptance + Miri verify. |
| `read_unaligned` on payload bytes UB | Low | All commands `Send + 'static`. Miri verifies. |
| `MaybeUninit<u8>` storage trips Miri | Low | PR #6391 demonstrated soundness. |
| `Bundle::component_ids` OnceLock cost | Low | ~30 ns/call cold-amortised. Phase 9 §9.10 migration. |
| `Bundle::for_each_component_bytes` callback panic ⇒ double-drop | **Low after C1' FIX (B4)** | ManuallyDrop wraps every element BEFORE any callback runs; panic leaks unfinished components (no Drop). Step 6 acceptance + Step 12 Miri test. |
| User Drop touches world | Medium | Document. Same contract as Resource::Drop. |
| `FunctionSystem::apply` re-entry | Low | APP4. Borrow checker enforces. |
| Turbofish-removal migration breaks Phase 8a/8b tests | Medium | Step 5 enumerates all call sites. |
| Arity 13+ stub release breakage | Low | W4 verified-then-defer. |
| CommandQueue 56 B × N systems | Low | 1000 systems = 56 KB. |
| Bundle::for_each_component_bytes lifetime | **Low after C1 + W4' (stack collector pattern)** | Bundle's locals (ManuallyDrop) live until function returns; callback completes ALL work (incl. terminal create_entity) inside. Miri verifies. |
| RawCommandQueue minting from `&mut self` | Low | `&raw mut self.bytes` is reference-free mint. Bevy parity. Miri. |
| Bundle::component_ids OnceLock+RwLock+HashMap cost | Low | ~30 ns; cold path. |
| **(NEW W1')** CommandQueue Send-bound mismatch with non-Send command | Low | `unsafe impl Send for CommandQueue` is sound because `Command: Send + 'static` is the trait bound on every `push<C>`. Phase 9 may add `unsafe impl Sync` if/when scheduler requires it. |

---

## 28. Out of scope (deferred)

(Unchanged from Round 2, plus:)
* **`ArchetypeRow` and `create_entity_with`** — REMOVED from Phase 8c/8d plan (W4'). If a future phase needs callback-driven piecewise insertion, design then.
* **`NotSendCommand` / `NotSendBundle`.** Phase 9 §9.4.
* **Const-fn-in-traits migration of Bundle::component_ids.** Phase 9 §9.10.
* **CommandQueue threshold-based shrink_to_fit.** Phase 9 §9.9 if measured.

---

## 29. Open questions for the critic — RESOLUTIONS

1. **GAT inference for turbofish removal — does Bevy's pattern work on stable Rust 1.85+ for two-lifetime GAT?** **RESOLVED.** Yes — §4.7 reproducer mirrors Bevy's macro shape; Step 2 acceptance test runs all three tests (incl. W2' elided form) before any further work.

2. **`Bundle::component_ids()` `&'static` mechanism — OnceLock or thread-local?** **RESOLVED.** OnceLock<RwLock<HashMap<TypeId, &'static [ComponentId]>>> — §12.3. ~30 ns/call. O1' documents Box::leak memory cost as bounded by N_BUNDLE_TYPES × 16 B (typical < 2 KB).

3. **`CommandQueue::apply` panic-recovery semantics — drain recovery first, or only on Drop?** **RESOLVED (C2' REVISED).** Bevy mirror: recovery is OPAQUE between apply calls. Only the catch_unwind Err branch in `apply_or_drop_queued` touches it: on top-level (`start == 0`) panic, recovery is appended to bytes IN THE SAME CALL via the Err branch. Next apply walks bytes (no special-case). Step 0.5 prepend (Round 2) was WRONG and has been DELETED. Step 8 acceptance tests `command_queue_recovery_opaque_between_applies` + `command_queue_top_level_recovery_reabsorbed_on_panic` lock this.

4. **`Bundle::write_into` lifetime cast soundness.** **RESOLVED.** No transmute (C1 FIX — callback API). C1' refined panic safety via ManuallyDrop-upfront (B4 invariant). W4' simplified the SpawnCommand::apply pattern to stack-collector + existing `create_entity` (no new EcsMaster API).

5. **`System::apply`'s re-entrancy hazard.** **RESOLVED (W8 / APP4).** Borrow checker enforces.

6. **`Commands::spawn` with empty Bundle.** **N/A** — arity 0 not implemented in 8d (arity starts at 1).

7. **Turbofish test cleanup.** **RESOLVED.** Strip turbofish from existing tests.

8. **(NEW C1')** **Bundle callback panic safety.** **RESOLVED.** ManuallyDrop wraps every element UPFRONT (B4 invariant). On callback panic mid-iteration: unfinished components leak (no Drop). Step 6 acceptance + Step 12 Miri verify.

9. **(NEW W1')** **CommandQueue Send/Sync.** **RESOLVED.** Explicit `unsafe impl Send for CommandQueue` documented + justified by `Command: Send + 'static` bound. `!Sync`. `RawCommandQueue: !Send + !Sync` (raw ptrs; intentional). §10.1 + §19 table.

10. **(NEW W3')** **Panicker retry vs skip.** **RESOLVED.** SKIP. Cursor advances BEFORE consume_and_drop's `cmd.apply` runs (in `consume_and_drop_glue`, §10.4); panicker is past the recovery window. Step 8 acceptance test `command_queue_panic_skips_panicker_runs_rest_on_redrive`.

11. **(NEW O3')** **System::apply / SystemParam::apply safety status.** **RESOLVED.** Both are SAFE methods. Caller holds `&mut EcsMaster` exclusively; no `unsafe` qualifier needed. APP1' invariant.

---

## 30. References

(Unchanged from Round 2.)

---

## Plan readiness checklist — Round 3 self-check

### Plan structure
- [x] Changes from Round 2 section at top
- [x] Goal stated in perf + functional terms
- [x] Target metrics concrete; W5 split applied; O2' first-call row added
- [x] Every architectural decision has perf/cache/parallelism justification
- [x] Each alternative has a reasoned rejection
- [x] Trade-offs honestly listed

### Round 3 critical fixes
- [x] **C1'** ManuallyDrop-upfront panic safety in Bundle::for_each_component_bytes (§12.3 + B4 invariant + Step 6/12 tests)
- [x] **C2'** "Step 0.5" prepend recovery DELETED (§10.3 + §21.2 + §29 Q3); panic_recovery OPAQUE between calls
- [x] **C3'** §12.3/§12.4 single canonical code block; dead `static IDS:` removed; FnMut/FnOnce mismatch resolved (FnMut for repeated invocation)

### Round 3 important fixes
- [x] **W1'** CommandQueue Send (unsafe impl) + RawCommandQueue !Send + !Sync documented (§10.1 + §19 + CQ-SEND1)
- [x] **W2'** Third reproducer test `closure_with_elided_param_type_compiles` for fully-elided closure form (§4.7 + Step 2 acceptance)
- [x] **W3'** Cursor advance BEFORE consume_and_drop's apply; panicker SKIPPED on redrive (§10.4 + Step 8 tests)
- [x] **W4'** ArchetypeRow + create_entity_with DROPPED from plan; SpawnCommand::apply uses stack-collector + existing create_entity (§12.4)
- [x] **W5'** Dead `static IDS:` block removed (subsumed by C3')

### Round 3 optional fixes
- [x] **O1'** Box::leak memory cost documented as bounded by N_BUNDLE_TYPES × 16 B (§12.3)
- [x] **O2'** First-call cumulative row in §1.2 (≈ 1.2 µs) + bench `bench_run_closure_once_first_call_cold`
- [x] **O3'** System::apply + SystemParam::apply both SAFE methods — APP1' invariant; §9.5 rustdoc

### Carryover from Round 2
- [x] All Round 1 critical fixes (C1-C6) intact
- [x] All Round 2 important fixes (W1-W8) intact
- [x] All Round 2 optional fixes (O1-O5) intact

### Data structures
- [x] Each field has type + role comment
- [x] `#[repr(...)]` specified where it matters
- [x] Hot/cold split applied
- [x] Struct size known and justified
- [x] Padding for false sharing N/A — single-threaded
- [x] Send / Sync for every type stated in §19 table

### API
- [x] Public API minimal (W4' shrunk by dropping ArchetypeRow + create_entity_with)
- [x] No internal types leak
- [x] Lifetimes explicit
- [x] No `dyn Trait` on hot path
- [x] Generics where specialization needed

### Multithreading
- [x] Model explicit (single-threaded in 8c+8d)
- [x] Atomics with memory ordering specified — none added
- [x] `Send`/`Sync` for types specified (W1')
- [x] Phase 9 implications documented

### Correctness
- [x] Edge cases enumerated
- [x] Drop order discussed (B4 ManuallyDrop)
- [x] Invariants for `unsafe` blocks stated
- [x] Phase 8a/8b invariants preserved
- [x] Panic-safety contract explicit (B4 + CQ4)

### Integration
- [x] Affected modules listed
- [x] Changes in existing APIs explicit (W4' — NO new EcsMaster method)
- [x] Compatibility with Arena / ComponentPool / UnitId verified
- [x] Implementation plan broken into 13 steps
- [x] `FnOnceSystem` removal mapped

### Validation
- [x] Unit tests specified (incl. new C1'/C2'/W3' tests)
- [x] Property tests specified
- [x] Benchmarks specified (W5 split + O2' first-call)
- [x] `debug_assert!` invariants specified (incl. SpawnCommand::apply arity + `count == arity` post-callback per W3'')
- [x] Miri test suite specified (incl. C1' double-drop verifier)
- [x] Phase 8b B1 lesson honored (runtime panic; W4 verified-then-defer)

End of Round 3 plan.
```

---

## Round 3 plan delivered.

The full revised plan above replaces `D:\claude\BoykoEngine\docs\PHASE-8CD-INTOSYSTEM-COMMANDS-PLAN.md`. Orchestrator should save it to the same path.

**Key changes in Round 3:**

- **C1' (Bundle panic safety):** Adopted ManuallyDrop-upfront pattern. Every destructured tuple element wraps in `ManuallyDrop<T>` BEFORE any callback runs. On mid-iteration callback panic, unfinished components LEAK (no Drop) instead of double-dropping with archetype-side ownership. New invariant **B4** documents the leak-vs-UB tradeoff. Miri test `miri_bundle_for_each_panics_no_double_drop` added.

- **C2' (apply Step 0.5 DELETED):** Round 2's "prepend `panic_recovery` to `bytes` at the start of every apply" was architect-invented and diverged from Bevy. Deleted entirely. `panic_recovery` is OPAQUE between apply calls; only the catch_unwind Err branch touches it (top-level `start == 0` ⇒ `bytes.append(panic_recovery)` IN THE SAME PANICKING CALL). Test `command_queue_recovery_opaque_between_applies` locks this so future refactors can't reintroduce the bug.

- **C3' (draft cleanup + FnMut/FnOnce):** Single canonical code block per section. Dead `static IDS:` removed (subsumed by `bundle_slot_for` helper). Resolved the FnMut/FnOnce confusion — the trait bound MUST be `FnMut` because the callback is invoked N times per bundle (FnOnce only allows one call). Updated everywhere.

- **W1' (Send/Sync):** Explicit `unsafe impl Send for CommandQueue` with justification (all enqueued commands satisfy `Command: Send + 'static`). `RawCommandQueue: !Send + !Sync` (raw NonNull pointers; intentional — transient stack value). New invariant **CQ-SEND1**.

- **W2' (third reproducer test):** Added `closure_with_elided_param_type_compiles` testing fully-elided `|p: StubParam|` form. This is the canonical user-facing shape (`|q: Query<&Position>|`); validates the headline ergonomic promise.

- **W3' (cursor advance discipline):** Cursor advances PAST the command's bytes INSIDE `consume_and_drop_glue` BEFORE `cmd.apply(world)` runs. On panic, `local_cursor` is already past the panicker; recovery range `bytes[local_cursor..bytes.len()]` excludes the panicker. Panicker is SKIPPED on redrive (matches Bevy).

- **W4' (ArchetypeRow dropped):** Round 2's undefined `ArchetypeRow<'a>` + `create_entity_with` API REMOVED from the plan. SpawnCommand::apply uses a stack-allocated `[MaybeUninit<(ComponentId, &[u8])>; 4]` collector inside `for_each_component_bytes`; on the terminal callback invocation (count == arity), it calls existing `EcsMaster::create_entity(archetype_id, &slots[..count])` WHILE the bundle's ManuallyDrop locals are still alive (lifetime soundness preserved).

- **O1'-O3' polish:** Box::leak memory cost documented (~2 KB max). First-call cumulative cost row added to §1.2 (≈ 1.2 µs). System::apply + SystemParam::apply both confirmed as SAFE methods via new APP1' invariant.

**Relevant file paths:**
- Plan output target: `D:\claude\BoykoEngine\docs\PHASE-8CD-INTOSYSTEM-COMMANDS-PLAN.md`
- Round 2 source: same path (will be overwritten)
- Phase 8a tuple_impl reference: `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\params\tuple_impl.rs`
- Phase 7 create_entity signature reference: `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\ecs_master\ecs_master.rs`
- FnOnceSystem source to delete: `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\fn_once_system.rs`
- HRTB reproducer target (3 tests in Round 3): `D:\claude\BoykoEngine\crates\boyko_ecs\tests\hrtb_reproducer.rs`

Sources:
- [Bevy `command_queue.rs` source — apply_or_drop_queued + panic_recovery semantics](https://raw.githubusercontent.com/bevyengine/bevy/main/crates/bevy_ecs/src/world/command_queue.rs)
- [Bevy `bundle.rs` source — Bundle write_components + ManuallyDrop discipline](https://raw.githubusercontent.com/bevyengine/bevy/main/crates/bevy_ecs/src/bundle.rs)
- [Bevy PR #4863 — CommandQueue panic recovery design](https://github.com/bevyengine/bevy/pull/4863)
- [std::mem::ManuallyDrop — Rust std docs](https://doc.rust-lang.org/std/mem/struct.ManuallyDrop.html)
- [Lifetime elision under HRTB — Rust reference](https://doc.rust-lang.org/reference/lifetime-elision.html)