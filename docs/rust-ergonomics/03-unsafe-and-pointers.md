# §3 Ownership, unsafe, pointers and failure paths — ERG-20 · 22 · 24 · 43 · 45 · 26 · 28

Part of [RUST-ERGONOMICS.md](../RUST-ERGONOMICS.md). Cost verdicts cite [EVIDENCE.md](EVIDENCE.md).
Code is cited by file path and item name, never by line number. (The former §4 — errors, panics
and assertions — is merged here: a release assert exists to guard an `unsafe` write, and a cold
panic helper is where a hot function's failure path goes.)

The theme: `unsafe` is free at runtime and expensive to review; the success path pays nothing
for the failure path. Every rule here either makes a block cheaper to AUDIT or moves work off the
hot path — and two of them (ERG-43, ERG-45) put ownership and memory-ordering facts into the
program where a comment used to carry them.

---

### ERG-20 — Every `unsafe` block has a `// SAFETY:` as the last thing before it naming who establishes each fact; every `unsafe fn` has `# Safety` and its inner block forwards to it; one discharged obligation per block; raw base accessors are `pub(crate)`

**Binding:** MUST (placement; `# Safety`; the forwarding form — CLAUDE.md principle 8 made
lint-checkable); SHOULD (one obligation per block; safe arithmetic outside; the content bar) ·
**Cost:** ZERO-COST (comments and block boundaries emit nothing) · **Guarantee level:**
language; lint behaviour measured (EV-57)

**Rule.** The delta over CLAUDE.md, which already binds the comment:
- The comment is the LAST thing before `unsafe {` — no statement in between. That is exactly what
  `clippy::undocumented_unsafe_blocks` checks (EV-57), which is what makes the MUST mechanical.
- Each clause is a FACT and its source — a `# Safety` precondition, a preceding release `assert!`,
  a type invariant, a scheduler gate — with the decision id (`D5`, `U11`, `SEND2`) where one exists.
- An `unsafe fn` documents the contract ONCE in `/// # Safety`; the block inside reads
  `// SAFETY: forwarded to the caller; see method doc.` — and edition 2024's
  `unsafe_op_in_unsafe_fn` (warn by default) is what keeps that block from disappearing.
- When two operations rest on different facts, split them so each carries its own clause; index
  arithmetic, `len` updates and branches on safe values go outside. Keep the two writes of a swap
  together (panic safety).
- Anything that leaks the raw base (`as_ptr`) is `pub(crate)`, so `grep` enumerates every way out.

**Before** — `// SAFETY: pointer is valid` / `unsafe { &*self.deque }`; and
`crates/boyko_ecs/src/ecs/memory/vm_column.rs`, `VmColumn::swap_remove`: one block, three
pointer operations, a branch and `self.len = last`, under one monolithic comment.

**After** — `crates/boyko_threadpool/src/tls.rs`, `WorkerLane::deque` (the deposit site, the drop
order, the `!Sync` fact and the D5 id, each checkable). The forwarding form, as a spelling:

```rust
/// # Safety
/// `idx < self.len` — the caller's proof, established by <who>.
pub unsafe fn row_unchecked(&self, idx: usize) -> *mut T {
    // SAFETY: forwarded to the caller; see method doc.
    unsafe { self.base.as_ptr().add(idx) }
}
```

(The tree's one instance of that spelling, `scope.rs`'s `SharedPtr::as_ref`, is written
correctly as a forwarding block — and is a BEFORE for ERG-06, because the `&'a` it forwards has
a free lifetime. The comment form is right; the signature it sits on is not, and a well-formed
comment must not be read as a well-formed shape.) For `swap_remove`:

```rust
// SAFETY: `index < len` (release assert above) ⇒ the slot is in the committed prefix; `T: Copy`.
let removed = unsafe { self.base.as_ptr().add(index).read() };
if index != last {
    // SAFETY: `last = len - 1 < len`, same prefix argument; `&mut self` ⇒ exclusive.
    unsafe { let v = self.base.as_ptr().add(last).read(); self.base.as_ptr().add(index).write(v); }
}
self.len = last;   // safe: plain field store
```

**What it buys.** A comment that repeats the expression in English makes the block look reviewed;
one carrying the deposit site and the rule id can be checked line by line. When an obligation later
becomes false, the text says WHICH clause covered it.

**Verified.** EV-57: six placements against clippy 1.97.1 — a `debug_assert!` between comment and
block is REJECTED; a blank line is accepted. Census: 2391 `unsafe {` vs 2267 `// SAFETY:` lines;
the gap is measurable only by the lint (OPEN 4 — `unsafe_op_in_unsafe_fn = "deny"` and
`undocumented_unsafe_blocks = "warn"` belong in the workspace lint table beside `disallowed_types`,
the same move the 2026-07 audit made; not while other agents hold `crates/`).

**Exceptions.**
- Macro-generated `unsafe` cannot carry a per-site comment; the macro's doc carries it once.
- `multiple_unsafe_ops_per_block` has false positives in nested closures; `warn`, not `deny`.

---

### ERG-22 — A raw pointer is minted with `&raw const` / `&raw mut` where a reference must not be, owned as `NonNull<T>` with an explicit variance marker, and nullable as `Option<NonNull<T>>` — never a null sentinel tested by hand

**Binding:** MUST · **Cost:** ZERO-COST (EV-37, EV-05, EV-69) · **Guarantee level:** language
(RFC 2582; `Option<NonNull>` niche and all-zero `None` are std guarantees); measured

**Rule.**
- `&place as *const T` materialises a reference and casts it — a retag under Tree Borrows, with
  the reference's permission inherited by the pointer. `&raw const place` (or `addr_of!`, the same
  operation) creates no reference. Use it whenever the pointer will be written through, the place
  is concurrently accessed, or it may be unaligned or uninitialised; project fields as
  `&raw const (*p).field`. This governs the FORM, not the spelling: a diff rewriting `addr_of!`
  to `&raw` in the tree's Tree-Borrows code is a zero-diff change and is refused.
- An owning field is `NonNull<T>` plus the ERG-05 marker that states variance and auto-traits.
- A field that may be empty is `Option<NonNull<T>>`: `None` IS the null pointer, the type is one
  word, `mem::zeroed()` decodes to `None`, and `let base = col.ptr?;` PRODUCES the non-null value
  the following `unsafe` arithmetic needs — where `if col.ptr.is_null()` only checks it.

**Before** — `crates/boyko_ecs/src/ecs/core/archetype/archetype.rs`, `Column`:

```rust
#[repr(C)] pub struct Column { pub(crate) ptr: *mut u8, pub(crate) stride: u32, pub(crate) _reserved: u32 }
pub const fn is_null(&self) -> bool { self.ptr.is_null() }   // "first check on every fast-path lookup" —
                                                              // nothing ties having done it to the pointer arithmetic that follows
```

`crates/boyko_rhi_vulkan/src/ffi.rs`: `VkBuffer(pub u64)` with `pub const NULL` and `is_null()`
checked at some sites and not others. And `let deposit = WorkerDequeDeposit::new(pool, &deque as
*const _)` — a `&Worker` minted and cast.

**After** — `crates/boyko_threadpool/src/worker.rs`, `worker_main`: `WorkerDequeDeposit::new(…,
&raw const deque)`; `crates/boyko_threadpool/src/scope.rs`, `Scope::shared: NonNull<ScopeShared>`
with `PhantomData<&'scope mut &'scope ()>` ("copies the pointer WITHOUT retagging the pointee");
`crates/boyko_ecs/src/ecs/core/ecs_master/component_api.rs`, `addr_of!((*p).columns)` — "a
PROJECTION, never a struct-wide reference" (BUG-MIGRATE-TB-1). For `Column`:

```rust
#[repr(C)] pub struct Column { ptr: Option<NonNull<u8>>, stride: u32, _reserved: u32 }
pub const fn null() -> Self { Self { ptr: None, stride: 0, _reserved: 0 } }   // the all-zero reset survives
let base = col.ptr?;                                                            // the proof, not a re-check
unsafe { *(base.as_ptr().add(row * col.stride as usize) as *const u32) }
```

**What it buys.** The transient reference is an aliasing event assembly cannot show and Miri can
(the C1 downgrade, Phase 9.2 Candidate U). Non-nullness moves into the type; an "unset" state
costs no extra word; and the `?` ties the check to the arithmetic that depends on it.

**Verified.** EV-37: `&raw const` and `&x as *const _` fold to one symbol. EV-05:
`size_of::<Option<NonNull<u8>>>() == 8`. EV-69 (this pass): `Column` with `*mut u8` and with
`Option<NonNull<u8>>` — size 16, align 8, `offset_of!(stride) == 8` for both (the file's existing
layout gates pass unchanged); `mem::zeroed().ptr.is_none()` folds to a constant `true`; the
bounds-checked / null-tested / scaled-read `read` bodies are 0 normalised-diff lines. EV-59:
`is_null_rust = is_null_c` for the `Option<NonZeroU64>` handle form.

**Exceptions.**
- `NonNull` conveys non-nullness ONLY — nothing about validity, alignment or aliasing; it must
  not shorten a SAFETY comment. It is `!Send + !Sync`; the `unsafe impl Send` it forces carries
  the real argument (nine lines in `scope.rs`).
- **The FFI split:** a struct the OS or driver WRITES may hold `Option<NonNull>` (a written `0`
  decodes as `None`) but never a bare `NonNull` (a written `0` is instant UB). Bare `NonNull` only
  for handles the Rust side minted.
- Do not "fix" `WorkerLane::deque(&self)` to a by-value receiver: the `&self` tie is the stronger
  signature, and the receiver shape proves nothing about aliasing (EV-38 is codegen identity only).
- A by-value cell that reaches a task body has raw / `NonNull` fields only (ERG-06 clause 1).

---

### ERG-24 — Reach for the slice API (`split_at_mut`, `chunks_exact_mut`, `iter_mut`) before raw-pointer arithmetic for disjoint access; a raw pointer only for a named aliasing reason with its id, or with a measurement at the site

**Binding:** MUST · **Cost:** ZERO-COST — on rustc 1.97.1 the two forms vectorise alike and time
alike (EV-60, EV-70); the ledger's "25 % faster" (EV-22) no longer reproduces · **Guarantee
level:** measured

**Rule.** Two mutable regions of one buffer are `let (lo, hi) = buf.split_at_mut(mid)`; a stride
is `chunks_exact_mut(n)`. A raw pointer is minted instead in exactly two cases:
- (a) **No slice expresses the access** — per-row provenance handed to N workers by colouring
  (ERG-07's `row_ptr`) — or the slice form was MEASURED slower at the site and the number is
  written there.
- (b) **Aliasing.** The access is a raw PROJECTION whose `// SAFETY:` names the wider place it
  avoids, the live pointer that would conflict, and the id (`U11`, `F4`, `BUG-MIGRATE-TB-1`,
  `Phase 9.2 Candidate U`). Case (b) is exempt from the speed test: it is ERG-22's form on a
  projection, and replacing it with the "safe" `&self.slots[i]` is the class of Miri defect this
  repository has already fixed.

**Before / After** —

```rust
let base = buf.as_mut_ptr();                                  // Before
for i in 0..half { unsafe { *base.add(i) += *base.add(i + half); } }
let (lo, hi) = buf.split_at_mut(half);                        // After
for (a, b) in lo.iter_mut().zip(hi) { *a += *b; }
```

In-tree case (b): `archetype_bundle.rs`, `slot_ptr_mut` ("going through `Index` would
materialise a transient `&MaybeUninit<Archetype>` whose borrow-stack pop could later
retag-conflict …", U11); `component_api.rs`'s `addr_of!((*p).columns)`; `dispatcher_token.rs`'s
`WorldView` ("NEVER forms a struct-wide `&EcsMaster`").

**What it buys.** Review budget: the slice form has no SAFETY obligation, and it is never slower.
The previous draft claimed a speed win; that claim is withdrawn — the rule stands on
reviewability and on being at worst equal.

**Verified.** EV-70 (this pass): `f32` halves — `split_at_mut` 40 instructions with 2 `addps`,
raw pointers 34 with 2 `addps`; BOTH vectorised. EV-60 (runtime-cost guard, three runs):
0.078–0.085 vs 0.082–0.085 ns/elem on `f32`; 1.062 vs 1.083–1.102 on 64-byte components — within
noise. EV-22's 2026-08 result (77 scalar instructions, 25 % slower) is kept in the ledger as
superseded.

**Exceptions.**
- The colour-partitioned shape is case (a) by construction and carries "disjointness comes from the
  colouring" in its SAFETY.
- The check on a diff: does the raw projection's SAFETY name the wider place, the conflicting live
  pointer, and the id? If yes, ERG-22 governs and the slice form is the WRONG form.

---

### ERG-43 — An action that must be undone is a `Drop` guard; a borrow conflict on `&mut self` is `mem::take` / `mem::replace`, never a clone or a side store; a value whose bytes have been copied out is `ManuallyDrop`

**Binding:** MUST (the guard, for any undo — a TLS depth, a cursor write-back, a suppression flag,
a raw allocation freed on early return); SHOULD (`mem::take` / `replace` / `swap` over `clone` /
`RefCell` / a parallel `Vec`); MAY (`ManuallyDrop` / `MaybeUninit`, with layout gates) · **Cost:**
the guard's normal path is IDENTICAL; it adds an unwind landing pad (EV-62); `mem::take` of a
`Vec` field is deleted by LLVM (EV-65); `replace` of a large inline array COSTS two `memcpy`s
(EV-65) · **Guarantee level:** language (drop order, drop-on-unwind); measured

**Rule.**
1. The guard type has a private field so it cannot be built elsewhere, one `enter` / `new`,
   `#[must_use]` with a message (ERG-11), `#[inline]` on both `enter` and `drop`, and it is
   DECLARED AFTER anything its drop body reads — locals drop in reverse declaration order, and
   drops run during unwinding, which is what makes the read valid on both exits. Bind it `let _g =`;
   `let _ =` drops it on the same line.
2. When a `&mut self` method must hand a field's contents to something that also needs `self`,
   move the field out with `mem::take` (needs `Default` = empty-but-valid) or `mem::replace`,
   work on the owned local, move it back. The retained capacity travels with it.
3. A value whose bytes were copied into storage the caller now owns is held in `ManuallyDrop<T>`
   (or moved out with `ptr::read` under `mem::forget`); storage not yet initialised is
   `MaybeUninit<T>`, so a forgotten field is a compile-time `assume_init` obligation rather than
   plausible garbage.

**Before** — a hand-written epilogue:

```rust
DEPTH.with(|d| d.set(d.get() + 1));
body();                                    // an early `return`, a `?`, or a PANIC skips the line below
DEPTH.with(|d| d.set(d.get() - 1));
```

**After** — `crates/boyko_threadpool/src/tls.rs`, `InSystemRunGuard` (`let _g =
InSystemRunGuard::enter();` in `System::run_unsafe`); `crates/boyko_ecs/src/ecs/core/commands/command_queue.rs`,
`CursorSync` — "the guard's Drop writes the local back on EITHER normal completion OR unwind — so
the Phase 12.5 panic-recovery semantics survive"; `tls.rs`, `WorkerDequeDeposit`, declared after
the `deque` parameter so it clears the slot before the deque drops. Clause 2: the P19 Tree-Borrows
fix (`mem::take` the command queue's buffers into a stack-local twin, apply, swap the survivors
home — `docs/archive/BUG-P19-TB-1-PLAN.md`) and the UI layout scratch protocol (GUI-P1 Decision 7).
Clause 3: `archetype_bundle.rs`'s slab constructor and the `MaybeUninit` assembly in
`brick_atlas.rs` (REF-27 form 4).

**What it buys.** Three defect classes this repository has been bitten by: the early exit that
skips the cleanup; the PANIC between set and unset that leaves a flag, cursor or depth permanently
wrong (in a threadpool where a task body may panic and a joiner runs a sibling system inline, this
is not theoretical); and a new exit path added later that forgets the epilogue. Clause 2 removes
the reason a C-trained author reaches for `RefCell`, a clone or a parallel `Vec` — all three banned.

**Verified.** EV-62 (this pass, `panic = unwind`): with an opaque `fn()` body the guard and the
manual epilogue have IDENTICAL normal paths (the same 20 instructions); the guard adds exactly a
7-instruction cleanup landing pad (`drop_glue`, `_Unwind_Resume`, `panic_in_cleanup`) plus an
exception-table entry — the cost IS the thing bought. With a body the compiler can see does not
unwind, the two fold to one symbol (`run_manual_nounwind = run_guard_nounwind`), at codegen-units 1
and 16. EV-65: `mem::take` of a `Vec` field around a helper call is the same 5 instructions as the
plain reborrow — LLVM deleted the move-out and move-back; `mem::replace` on a `[u8; 4096]` field
is two 4095-byte `memcpy`s and a 4136-byte frame with a `___chkstk_ms` probe. EV-59:
`run_manual = run_guard` alias in the finder's lab (a non-unwinding body).

**Exceptions.**
- A drop body that can panic while already unwinding aborts the process; drop bodies stay
  infallible. A commit-on-success is `ManuallyDrop` / `mem::forget` on the success path, not a
  `Drop` with a `bool`.
- Do not wrap a pairing with exactly one exit that cannot panic — that is a type to learn for no
  defect prevented (REF-32).
- The window between take and put-back is a state where the field is EMPTY and observable: if a
  hook, observer or re-entrant command can read it there, the empty value is a wrong answer. Pair
  it with a guard whenever a panic can occur in the window. Never `mem::take` a large inline
  array (EV-65 / EV-08).
- `MaybeUninit` is contagious (every reader goes through `assume_init_ref`); `ManuallyDrop` moves
  the drop obligation rather than removing it. Gate `size_of` / `align_of` of both wrappers over
  the payload (std guarantees them equal), and prefer the current `addr_of_mut!().write()` form
  where storage is always initialised before it is reachable.

---

### ERG-45 — An atomic's ordering is the WEAKEST that discharges a named happens-before, and the site names the operation it pairs with; `SeqCst` is never a default

**Binding:** MUST · **Cost:** a `SeqCst` STORE is a locked read-modify-write; `Release` /
`Relaxed` stores are plain stores; loads are identical at every ordering; RMWs are locked at every
ordering (EV-63) · **Guarantee level:** measured (x86-64); the model half is the C++11 memory
model

**Rule.** Every `store` / `load` / `fetch_*` / `compare_exchange` names, in its doc or SAFETY,
which operation it synchronises with and what is published or acquired — "`Release` here pairs
with the `Acquire` load in `is_drained`; publishes the task body written above". A safe method
that wraps an atomic (`register_task`, `complete_task`) carries that pairing in its doc, because
the method name says neither "atomic" nor which ordering. `Relaxed` only with a written reason it
synchronises nothing (a statistic, a counter read under another synchronisation). `SeqCst` only
where a store–load ordering across two locations is the argument (the Dekker-style wake decision),
and then usually as one `fence(SeqCst)` rather than on every access.

**Before** —

```rust
self.pending.store(v, Ordering::SeqCst);   // reads as "the safe one"; compiles; never wrong; a bus lock per store
```

**After** — `crates/boyko_threadpool/src/scope.rs`: `register_task` wraps `fetch_add(1, AcqRel)`,
`complete_task` wraps `unpark()` then `fetch_sub(1, AcqRel)` with a load-bearing ORDER note, and
`is_drained` wraps `load(Acquire)` — each documenting its pairing. `crates/boyko_threadpool/src/worker.rs`:
the one production `SeqCst` is a single `fence(SeqCst)` ("a local `mfence` on x86, no cache line
touched") on the wake path, with the reason beside it. Census of the crate: 29 `Acquire`, 10
`Release`, 8 `AcqRel`, 6 `Relaxed`, 1 `SeqCst`.

**What it buys.** The largest failure-B hole for a lock-free kernel: `SeqCst` is the ergonomic
default — it reads as safe, compiles, is never wrong, and nothing complains — and on x86-64 the
cost is one-sided and invisible in a load-heavy review. On a steal path that stores a deque index
per operation it is the difference between a store and a ~20–40-cycle bus lock. The pairing note
is what a lock-free reviewer must see and cannot get from the method name.

**Verified.** EV-63 (this pass): `store(v, SeqCst)` → `xchgq %rdx, (%rcx)`; `store(v, Release)`
and `store(v, Relaxed)` → `movq %rdx, (%rcx)`. `load` at `Relaxed`, `Acquire` and `SeqCst` →
the identical `movq`. `fetch_add` at `SeqCst` and `Relaxed` → the identical `lock xaddq`;
`fetch_add` whose result is unused → `lock incq` at both. `compare_exchange` and
`compare_exchange_weak` in a retry loop → the identical `lock cmpxchgq` (the strong/weak split is
a portability question — ARM LL/SC — not an x86 cost). EV-60 measured the same `xchgq` / `movq`
pair independently.

**Exceptions.**
- The loom shim (`crate::sync`) is a `cfg`-selected `pub use` (ERG-33); a wrapper around an
  atomic must live on the shimmed type, not around `core::sync::atomic` directly, or the loom
  models stop seeing it.
- `compare_exchange_weak` is preferred in a loop for portability; on x86 the two are one
  instruction and neither is a cost argument.

---

### ERG-26 — `expect("invariant: …")`, never `unwrap()`; `debug_assert!` by default; a release `assert!` only where a vanished check would silently corrupt, with that sentence at the site; a dynamic invariant no type can carry lives in a `#[cfg(debug_assertions)]` field

**Binding:** MUST · **Cost:** `expect` and `debug_assert!` ZERO-COST on the hot path (EV-48,
EV-47); a release `assert!` COSTS two hot instructions plus a cold block (EV-47), free inside a
vectorised fill loop (EV-58, EV-60) · **Guarantee level:** language / measured

**Rule.** The delta over CLAUDE.md (which binds the `expect` spelling): the message names what the
CALLER must have upheld, not what the container observed — "invariant: Index requires a live
entry; use get() for a fallible lookup", not "Index not found". Three cases for a check:
1. An invariant the surrounding code establishes: `debug_assert!(cond, "invariant: …")`.
2. A violation that would NOT crash but silently corrupt — truncate an index and alias a live
   handle, write past a frontier under `unsafe` from a length safe code supplied: a release
   `assert!`, and the doc at the site carries "a vanished check would …". `assert!(a == b)`, not
   `assert_eq!`, on a hot path — the latter pulls both `Debug` impls into the binary.
3. A dynamic property no type can carry (this token is used on the thread that minted it): a
   `#[cfg(debug_assertions)]` field plus a `debug_assert!`; the field does not exist in release,
   so the ERG-01 gate is cfg-split.

**Before** — `crates/boyko_utils/src/sparse_map/sparse_map.rs`, `impl Index`:
`.expect("Index not found in SparseMap")`. And `slot_to_u64` written with `debug_assert!`: a
`> 2^32` index is truncated in release and aliases a live handle, defeating the ABA guarantee with
no panic anywhere.

**After** — `crates/boyko_rhi/src/handle.rs`, `slot_to_u64`: "a **release-present `assert!`**
(NOT `debug_assert!`, plan C2/D6): a vanished check would silently truncate a `> 2^32` index and
alias a live handle"; `crates/boyko_ecs/src/ecs/memory/vm_column.rs`, `VmColumn::extend_exact`:
a release assert per element because `ExactSizeIterator::len()` is safe code that can lie (the
in-file `Lying<I>` test); `dispatcher_token.rs`: `#[cfg(debug_assertions)] owning_thread:
ThreadId`. `scope.rs`: `.expect("invariant: Box::into_raw never yields null")`.

**What it buys.** The cost of a check is two instructions; the cost of a missing one is a crash
(acceptable — that is the `debug_assert!` trade) or silent corruption (never). The sentence at
the site is what lets a reviewer tell which trade was made.

**Verified.** EV-48: `expect` and `unwrap` have identical hot paths; the message costs two cold
instructions. EV-47: `debug_assert!` folds to the assert-free body; a release `assert!` is `shrq
$32; jne` plus a cold block. EV-58: inside `extend_exact`'s vectorised fill the per-element compare
folds into the trip count; EV-60 closed the remaining case — on an opaque iterator whose `next()`
LLVM cannot see through, the per-element release assert timed identically. EV-10: the cfg field is
absent from the release type (8 bytes).

**Exceptions.**
- Tests and `#[cfg(test)]` may `unwrap()`; a proc-macro crate runs at build time.
- `#[should_panic(expected = "…")]` pins message text: grep before changing one.
- 973 of 1411 `expect(` sites carry the prefix and exactly one production `.unwrap()` exists
  (`boyko_macros/src/event.rs`); `clippy::unwrap_used` at `warn` would make the rule a diagnostic
  (OPEN 4).

---

### ERG-28 — Failure paths are cold and small: a formatting panic is a `#[cold] #[inline(never)] fn -> !`; a kernel error is a `Copy` enum with at most a `&'static str`; a FAILURE signal is `Result<(), FieldlessEnum>`, never `bool`, and a predicate stays `bool`; `#[inline]` on trivial cross-crate methods and `#[inline(always)]` only with the measurement line

**Binding:** MUST (cold helpers where a panic formats or names a type; `Result` over `bool` for a
FAILURE signal, by the decider below; the inlining discipline — principle 7 made mechanical); SHOULD (the `Copy` +
`&'static str` error shape, size-gated) · **Cost:** the hot function shrinks (EV-32);
`Result<(), Fieldless>` is one byte and the same instructions as `bool` (EV-68); widths per EV-54;
`#[inline(always)]` COSTS L1i when wrong · **Guarantee level:** measured / documented compiler
behaviour (EV-27)

**Rule.**
- A `panic!` that formats anything is a `#[cold] #[inline(never)]` helper returning `!`, so the
  call site is one `call` in a cold arm. Error enums are `Copy`, a tag plus at most a `&'static
  str`, constructed through a `#[cold]` associated function, `Display` under the same pair.
- A function that can FAIL returns `Result<(), E>` with a fieldless `E`: it is `#[must_use]` by
  construction, `?` stops at the FIRST failure, and the error names WHICH pool or row — a `bool`
  is none of those, and `success &= pool.swap_remove(row)` keeps mutating pools past the first
  refusal, leaving a torn bundle with an error that says nothing. The decider between a failure
  and a PREDICATE: `Result` when `false` is an abnormal outcome the caller must act on, or when
  the callee changed state before refusing; `bool` when both outcomes are normal and the name
  reads as a predicate. `crates/boyko_threadpool/src/worker.rs`, `unpark_one_idle(inner) -> bool`
  ("was a parked worker woken") stays a `bool` — `wake_after_push` drops the answer on purpose,
  because "nobody was parked" is a normal outcome, and a `Result<(), NobodyParked>` would only
  add a `let _ =` with a reason at its one caller; `try_place_on_idle_sibling -> Result<(),
  TaskHandle>` in the same file is the shape that earns `Result`, because the task must come back.
- `#[inline]` on trivial cross-crate and generic methods. Any `#[inline(always)]` carries, on the
  line above, what was measured (tool, date, bench, delta) and what the compiler did without it —
  or it is `#[inline]`. `#[cold]` obeys the same measurement rule.

**Before** — `crates/boyko_ecs/src/ecs/core/component/component_pool_bundle.rs`:

```rust
let mut success = true;
for pool in self.pools.iter_mut() { success &= pool.swap_remove(unit_index); }   // keeps going after a failure
if !success { return Err(EcsError::PoolSwapRemoveFailed); }                       // which pool? which row?
```

`crates/boyko_ecs/src/ecs/core/archetype/archetype_bundle.rs`: two `#[inline(always)]` with no
measurement line; `schedule_builder.rs`: an empty release no-op under `#[inline(always)]`.

**After** — `crates/boyko_ecs/src/ecs/core/iters/query/query.rs`, `query_single_panic_many`
(`#[cold] #[inline(never)] fn … -> !`); `crates/boyko_rhi/src/error.rs`, `RhiError` ("no owned
allocation, no erasure, no `Box`"); and

```rust
pub fn swap_remove(&mut self, row: RowIndex) -> Result<(), RowOutOfBounds>    // pool side
for (i, pool) in self.pools.iter_mut().enumerate() {                            // bundle side
    pool.swap_remove(row).map_err(|_| EcsError::PoolSwapRemoveFailed { pool: InlandPoolId(i), row })?;
}
// #[inline(always)]: measured <date> (<tool>, <bench>): without it rustc left this out-of-line in <caller> and the call cost <delta>.
```

**What it buys.** Principle 3's I-cache half: format-string setup and panic sequences leave the hot
function, which is what makes `expect("invariant: …")` affordable at 1411 sites. The `Result` form
gives the signal a type the compiler refuses to drop and the loop a reason to stop.

**Verified.** EV-32: 13 instructions with the helper vs 21 with inline `panic!`s; EV-60: 8 vs 18 on
a 64-byte component shape. EV-68 (this pass): `remove_c -> bool` and `remove_r -> Result<(),
Fieldless>` are the same 8 instructions differing only in `setb` / `setae`; `size_of::<Result<(),
Fieldless>>() == 1`; the bundle loop is 27 instructions in both forms, the `?` form exiting on the
first `Err`. EV-54: a `&'static str` payload costs 16 B over a bare tag; `Result<(), Box<dyn Error>>`
is a pointer over a heap allocation and a vtable. EV-27: rustc 1.97.1 inlines tiny and ~20-line
cross-crate bodies WITHOUT `#[inline]` and needs it only above its `cross_crate_inlinable`
threshold — the attribute is redundant below and load-bearing above; the three production
`#[inline(always)]` sites lack the line.

**Exceptions.**
- A generic cold helper monomorphises per `<D, F>` — deliberate, N cold copies; never per element.
- Where a `Result` crosses a per-element `?`, prefer a tag-only enum and keep the text in `Display`.
- Changing `swap_remove`'s return type touches its callers and CHANGES behaviour (stops early) —
  its own commit, with the bundle tests re-run. Report-only while `boyko_ecs` is under edit.
- Diagnostics that are not panics go through the class-typed code registry
  (`crates/boyko_log/src/codes.rs`), whose orphan / page / premature-emitter checks are the gate.
