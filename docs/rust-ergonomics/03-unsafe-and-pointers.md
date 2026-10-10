# §3 Ownership, unsafe, pointers and failure paths — ERG-20 · 22 · 24 · 43 · 45 · 26 · 28 · 46 · 48

Part of [RUST-ERGONOMICS.md](../RUST-ERGONOMICS.md). Cost verdicts cite [EVIDENCE.md](EVIDENCE.md).
Code is cited by file path and item name, never by line number. (The former §4 — errors, panics
and assertions — is merged here: a release assert exists to guard an `unsafe` write, and a cold
panic helper is where a hot function's failure path goes.)

The theme: `unsafe` is free at runtime and expensive to review; the success path pays nothing
for the failure path. Every rule here either makes a block cheaper to AUDIT or moves work off the
hot path — and three of them (ERG-43, ERG-45, ERG-48) put ownership, memory-ordering and
stream/timeline facts into the program where a comment used to carry them.

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

**Clause 5 (merged from ERG-09, second sweep) — a kernel trait whose implementors must uphold
an `unsafe` invariant is SEALED, and the site says which seal.** The same question this rule asks
of a block — who establishes the fact — asked of a trait: if a downstream `impl` can make the
kernel's assumption false, the trait is not left open. `mod sealed { pub trait Sealed {} }` in a
PRIVATE module is a hard seal; where a derive macro must emit the impl downstream the module is
`#[doc(hidden)] pub mod sealed` and the doc says "a discoverability boundary, not an enforcement".

*Before* — `crates/boyko_utils/src/bit_mask/bit_set.rs`, `BitInteger`: public and unsealed, so a
downstream `impl BitInteger for Foo { const BITS: usize = 1000; }` makes every
`debug_assert!(index < T::BITS)` in `BitSet` vacuous and shifts past the width.
*After* — `crates/boyko_ecs/src/ecs/core/app/plugins.rs`: `mod sealed { pub trait Sealed<Marker>
{} }`, `pub trait Plugins<Marker>: sealed::Sealed<Marker>`; for `BitInteger`, one
`impl sealed::BitIntegerSealed for u8 {}` per blessed type.
*Verified* — EV-11: sealed-with-marker and plain trait folded to one symbol; COMPILE-TIME ONLY.
*Exceptions* — a `pub use` of the sealed module silently unseals; with a `Marker` parameter the
supertrait must carry the SAME marker or the impls overlap again;
`crates/boyko_ecs/src/ecs/core/bundle/bundle.rs`'s first paragraph above `mod sealed` describes a
hard seal for a module that is `pub mod sealed` — doc-rot to delete on next edit. The same
mechanism is what closes the marker set in ERG-31 clause 3.

---

**Clause 6 (added by the third sweep) — each clause is labelled VALIDITY or SAFETY, and that is
the STOPPING RULE this rule was missing.** The MUST above says each clause is "a FACT and its
source" but gives no way to know when the comment is finished, so a block can look complete while
its load-bearing half is absent. Ralf Jung's distinction supplies the checklist. A **validity**
fact is one the compiler or the type already enforces — non-nullness via `NonNull`, alignment via
the ERG-01 gate or ERG-48 shape 1's marker, initialisedness via `MaybeUninit`, "no drop glue" via
a `Copy` bound, "no whole-buffer `&mut` exists" via a view type with no slice surface (ERG-07). A
**safety** fact is one only this module's privacy upholds — `row < len`, "this pointer came from
`alloc`", "no other worker writes this index". The review then terminates: *for every validity
fact, name the type or gate that carries it — if you can, DELETE the clause; for every safety
fact, name the module whose privacy upholds it — if you cannot, that is the defect.*
*Before* — `crates/boyko_physics/src/systems.rs`'s pass-2 block mixes an AVX2-availability fact
(validity, and now stale under ERG-46) with an in-bounds fact (safety) in one paragraph, so
neither is checkable on its own.
*After / measured on the biggest instance* — five blocks of `colored.rs` rewritten under the
template: **13 clauses; 7 are validity facts a type already discharges and are deletable; 6 are
safety facts, and ALL SIX name the coloring, which nothing inside `colored.rs` upholds** (EV-103).
That count is the value of the refinement, and it points straight at ERG-48 shape 2 — the six
unupholdable clauses are exactly what an `unsafe` marker bound carries instead.
*The second half — the module IS the trusted base.* This rule audits blocks; clause 5 audits
implementors; the third door is a SAFE fn in the same module that can write a private field an
`unsafe` block depends on. No lint and no grep finds it, and Jung's rule is that the scope of
`unsafe` ends at the next abstraction boundary — so module SIZE is a soundness budget. Measured as
(private fields an `unsafe` block relies on) : (SAFE fns in the module that can write them):
`component_pool.rs` **13 : 93**, `vm_column.rs` **8 : 37**, `archetype.rs` **13 pub fields : 82**,
`scope.rs` **8 + 5 pub : 31** (EV-103) — four to seven safe fns per field, i.e. the trusted base
is the whole module surface, not a handful of writers. A module that grows past that is not a
style question. SHOULD, and deliberately not a MUST: the number is a review prompt, not a gate,
and ERG-25 was cut once for being unenforceable prose (see the retired-ids table).

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

**Clause 4 (added by the second sweep; ⚠️ ITS ROUNDTRIP HALF IS WITHDRAWN by the third — see the
correction at the end of this clause) — an address is taken with `.addr()`; never `ptr as
usize`.** This rule governs how a pointer is MINTED and OWNED; the missing half is
how its address is EXTRACTED. `ptr as usize` erases provenance, and the erasure is silent. The
Strict Provenance APIs are stable (1.84) and `.addr()` is a token-for-token substitution:

```rust
let seed = splitmix64((wid as u64) ^ (shared.addr() as u64));       // no provenance exposed
let ok = base.addr().is_multiple_of(SIMD_BUFFER_ALIGN);             // an alignment test wants a NUMBER
```

*Before* — `crates/boyko_threadpool/src/scope.rs`, the two `splitmix64(… shared as usize as u64)`
hash seeds; `crates/boyko_ecs/src/ecs/memory/component_pool.rs`,
`(data_base.as_ptr() as usize).is_multiple_of(SIMD_BUFFER_ALIGN)` — three sites that only want a
NUMBER and get an erasure. There is no fourth site: FV-12's census found **zero** int-to-ptr casts
in boyko's own production code, which is why the roundtrip half of this clause is withdrawn below.
*Verified* — EV-83: four ICF aliases at codegen-units 1 and 16; the substitution is free.
*Exception* — where a pointer's address is compared for IDENTITY (`ptr::eq`), neither API is
needed and `ptr::eq` is the spelling.

⚠️ **CORRECTION, third sweep — the ROUNDTRIP half of this clause is withdrawn, and the "not
measured" sentence it carried was already false when it was written.** As adopted, the clause also
prescribed `expose_provenance` / `with_exposed_provenance` for a genuine pointer roundtrip and said
"the BENEFIT was not measured: this pass ran no Miri (OPEN 1)". Its sibling ledger had the
measurement: **`RUST-FRONTIER.md`'s FR-13 refuses exactly that pair**, on FV-12's Miri run — the
`as`-cast and the expose / with-exposed pair produce the SAME warnings and the SAME strict-mode
error (Miri's strict-provenance text names `ptr::with_exposed_provenance` itself as unsupported),
and only `AtomicPtr` + `map_addr` passes. FV-12 also found that boyko's own production code
contains **ZERO** int-to-ptr casts: the one real roundtrip site this clause named,
`tls.rs`'s `worker_lane_for_is_none_on_a_worker_of_another_pool`, is inside `#[cfg(test)]`. So the
roundtrip half had no production site and bought nothing, and it is deleted here rather than kept
as a superseded prescription. **The `.addr()` half is unaffected and stands** — EV-83's three
address-extraction aliases are the ones that matter, and they are all this clause now says. This
is the repository's own recorded failure class, a summary outliving its refutation, inside the
guide that names it; OPEN 1 is corrected in the index accordingly.

---

### ERG-24 — Reach for the slice API (`split_at_mut`, `chunks_exact_mut`, `iter_mut`) before raw-pointer arithmetic for disjoint access; a raw pointer only for a named aliasing reason with its id, or with a measurement at the site

**Binding:** MUST · **Cost:** on `f32` the two forms vectorise alike and time alike (EV-70;
EV-22 re-verified 2026-09-03); ⚠️ **on a 64-byte column the SLICE form is the one that fails to
unroll, and it is 1.4–2.3× slower while the column is L1-resident**, indistinguishable at L2 and
beyond (EV-22). The ledger's "25 % faster" for the slice form (2026-08) and "at worst equal"
(2026-09-02) are both withdrawn; the rule rests on review budget ALONE, and case (a) below is its
measured escape · **Guarantee level:** measured at three cache tiers

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

**What it buys.** Review budget: the slice form has no SAFETY obligation. ~~And it is never
slower … the rule stands on reviewability and on being at worst equal.~~ Withdrawn 2026-09-03: on
a 64-byte column at L1 it IS slower (EV-22). The rule stands on reviewability, and on the measured
fact that the cost is confined to a wide, L1-resident chunk — which is exactly what case (a)'s
"measured slower at the site, number written there" exists for.

**Verified.** EV-70 / EV-22 (re-verified 2026-09-03 at the shipped profile, codegen-units 16
and 1): `f32` halves — `split_at_mut` 60 instructions / 15 vector ops / 12 `ymm`, raw pointers
62 / 18 / 12, BOTH vectorised, and the timing sign flips with code placement between two binaries
(parity). 64-byte column — `split_slice_col` 23 instructions / 3 vector ops with a per-iteration
`subq $1; jb <panic>` and NO unrolling; `split_raw_col` 43 / 15, 4× unrolled, no panic site; the
raw form faster in 6 runs of 6 at L1 (2.33× and 1.42× in two binaries), the binaries disagreeing
at L2 (1.06× / 1.21×), inside the 25 % band at 64 MiB. EV-60's 64-byte parity (1.062 vs
1.083–1.102 at 256 KiB) was its one L2 point and is consistent with this; it had no L1 point.
EV-22's 2026-08 result (77 scalar instructions, the raw form 25 % slower) is REFUTED and kept
struck through in the ledger.

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
the guard's normal path is IDENTICAL when its `Drop` inlines — a one-line guard's does; a
`#[inline(never)]` `Drop` puts a call on the normal path — and it adds an unwind landing pad
(EV-62); `mem::take` of a
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

**Verified.** EV-62 (`panic = unwind`; re-verified 2026-09-03 at an explicit codegen-units 16
and 1): with an opaque `fn()` body the guard and the manual epilogue have IDENTICAL normal paths
(9 instructions, byte-identical); the guard adds a FOUR-instruction cleanup landing pad (`decl;
movq; callq _Unwind_Resume; ud2`) plus `.seh_handler` and an exception-table entry — the cost IS
the thing bought. ~~A 7-instruction pad with `drop_glue` and `panic_in_cleanup`~~ — that shape
reproduces only when the guard's `Drop` is `#[inline(never)]`, and THEN the guard also puts a
`callq <Guard as Drop>::drop` on the NORMAL path and grows the frame (`subq $32` → `$48`): the
"normal path is identical" sentence is true exactly when `Drop` inlines, which a one-line depth /
cursor guard's does. With a body the compiler can see does not unwind, the two fold to one symbol
(`run_manual_nounwind = run_guard_nounwind`) at 16 and 1 units and across a forced unit split. EV-65: `mem::take` of a `Vec` field around a helper call is the same 5 instructions as the
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
- The loom shim (`crate::sync`) is a `cfg`-selected `pub use` (ERG-31 clause 4); a wrapper around an
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

**Clause 4 (added by the second sweep) — a third assertion tier lives on a cargo FEATURE, not
on the profile.** This rule's two runtime tiers are both pinned to `debug_assertions`, and that
axis also moves layout (every `#[cfg(debug_assertions)]` field — case 3 — and therefore every
timing). A campaign that wants a release-layout, release-speed build which still trips on a
violated invariant — a bit-determinism run, a scalar-oracle differential — cannot get it from
either tier. glam ships exactly this as `glam_assert` / `debug-glam-assert`:

```toml
[features] boyko-assert = []   debug-boyko-assert = []
```
```rust
macro_rules! boyko_assert { ($($t:tt)*) => {
    #[cfg(any(feature = "boyko-assert", all(debug_assertions, feature = "debug-boyko-assert")))]
    { assert!($($t)*); } }; }
```

*Verified* — EV-88: with `--features boyko-assert` in the RELEASE profile the guarded function
goes 22 → 35 instructions and gains exactly one panic site; without it, 22 and none; the
`#[cfg(debug_assertions)]` size gate is unchanged in both. **The tier changes checks, not
layout** — which is the whole point. The features owe ERG-31 clause 4's `compile_error!` pairs and a
banner like any other axis. Sites: `crates/boyko_math`'s quaternion / matrix constructors, which
have no normalisation guard today (glam's own default), and the `boyko_physics` scalar-oracle
differentials, which today choose between a debug build (checks on, layout different, untimeable)
and release (checks gone).

**Clause 5 (added by the second sweep, MAY) — a debug-only field may be wrapped so the host code
is `cfg`-free.** Case 3 says WHERE a `#[cfg(debug_assertions)]` field may live and ERG-01's
exception says such a type has two sizes to gate. A `MaybeDbg<T>` — `T` in debug, `PhantomData<T>`
in release, with an `Option`-like `as_ref()` — writes that gate ONCE instead of once per host
struct, and makes both configurations type-check on every build instead of only the one being
compiled (Bevy's `MaybeLocation`). *Verified* — EV-88: `MaybeDbg<u64>` is 0 bytes in release and
8 in debug; the host token's size equals the raw-`cfg`-field form in both; the check through
`as_ref()` and the raw `cfg` field are an **ICF alias**. Sites: the five `#[cfg(debug_assertions)]`
fields in `boyko_ecs` — `system/dispatcher_token.rs` (×2), `resources/nonsend_resources.rs`,
`system/unsafe_ecs_cell.rs` (whose own comment records that the field is why `repr(transparent)`
was impossible), `asset/path_index.rs`. A MAY: five sites is not a pattern, and the wrapper is a
name to learn (REF-29's neighbourhood) — it earns its place only where the host struct also has
a layout gate to write.

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

**Clause 5 (added by the second sweep) — a branchless SELECT is a directive, and owes the same
measurement line as `#[inline(always)]`, taken on BOTH predicate distributions.**
`core::hint::select_unpredictable` (stable 1.88; compiled here on 1.97.1) asks for a `cmov`
without inline asm, and std's own doc carries the falsifier: *"might decrease performance if the
condition is well-predictable"*. There are zero uses in this workspace and no site with a measured
misprediction rate, so it arrives WITH a row or not at all. *Verified* — EV-90: on an arithmetic
select (`acc += if d < r { hit } else { miss }`) the hint is an **ICF alias with the plain `if`** —
it buys nothing, because LLVM already blends; on a two-READ select (`if p { a[i] } else { b[i] }`)
~~it is 140 instructions and 12 branches against 343 and 54, and it runs 3.2–9.0× faster on a
random 50/50 predicate and 0.84–1.48× on a sorted one — a 19 % LOSS in one of three runs~~
**(re-verified 2026-09-03, EV-90)** the instruction counts are CODEGEN-UNIT-DEPENDENT (at the
shipped 16 units the select form is the BIGGER one, 99 against 35; at 1 unit 36 = 36) and are not
carried; the random-predicate win is **1.01× at L1 (none), 2.55× at L2 (real, ranges disjoint),
1.02× beyond the LLC (none)**; the sorted predicate is 1.00× / 1.02× / 1.00× — no loss and no win
at any scale. The measurement line at the site therefore names two distributions AND the working
set: the hint buys something only for a genuinely unpredictable branch over data that is L2- but
not L1-resident, and nothing on either side.

---

### ERG-46 — An `unsafe` whose only obligation is a TARGET FEATURE is deleted: the caller carries `#[target_feature]`, the kernel is a safe fn, and the island has exactly one boundary `unsafe`

**Binding:** MUST for a new SIMD kernel; SHOULD for migrating an existing one · **Cost:**
ZERO-COST (EV-77) · **Guarantee level:** language (`target_feature_11`, stable since Rust 1.86);
measured (codegen)

**Rule.** Since Rust 1.86 a SAFE `#[target_feature(enable = "…")]` fn is callable with **no
`unsafe`** from a caller whose attribute-declared features are a superset. So an `unsafe` block
whose `// SAFETY:` says nothing but "the callee is `#[target_feature]`" is not a fact to document
— it is a block to delete. The shape is an ISLAND: every fn in the SIMD module carries the
attribute and calls its neighbours plainly; the `unsafe` survives only where a genuine pointer
obligation survives (`_mm256_loadu_ps` and friends stay `unsafe fn` because of the pointer, not
the feature); and ONE boundary `unsafe` at the island's entry carries the one true sentence.

```rust
// SAFETY: cfg(target_feature = "avx2") holds for this crate — the build baseline is
//   x86-64-v3 (.cargo/config.toml, owner ruling 2026-09-02) — so the island's declared
//   features are present on the executing CPU.
unsafe { box_sdf_manifold_avx2(..) }
```

**Before** — `crates/boyko_physics/src/systems.rs`, `box_sdf_manifold_avx2`: two `unsafe` blocks
(pass 1 and pass 2) each spanning loads, a call to the SAFE `sdf_simd::sdf_edit_list_x8` and a
store, under a SAFETY paragraph containing the sentence **"`sdf_edit_list_x8` is
`#[target_feature(enable = "avx2")]`, so it must be called from an `unsafe` block on stable"**.
That sentence is stale: it must be called from an unsafe block from a NON-FEATURED caller, and
`box_sdf_manifold_avx2` is free to stop being one.
`crates/boyko_ecs/src/ecs/core/schedule/bitset_intersects.rs`, `bitset_intersects_avx2`, is the
other half of the same defect: an `unsafe fn` whose **entire** `# Safety` section is *"the caller
must guarantee that the executing CPU supports AVX2"* — a contract the attribute already enforces,
re-stated at its one caller in a six-line SAFETY block.

**After** — `crates/boyko_physics/src/sdf_simd.rs` (`sdf_edit_list_x8`),
`crates/boyko_physics/src/solver/simd.rs` and `solver/colored.rs` already spell their kernels as
SAFE `#[target_feature]` fns. **The tree is half-migrated, which is the worst state**: the safe
half is right, the calling half still writes the old prose, and the prose is now false.

**What it buys.** It removes a whole CLASS of `unsafe` blocks and one `unsafe fn` contract from
the review budget, and it removes the specific hazard that a stale feature sentence hides a real
obligation: a reviewer who reads "so it must be called from an unsafe block" stops reading, and
the pointer facts in the same block go unchecked.

**Verified.** EV-77: the plain caller and the island-plus-boundary are **14 instructions each, 0
normalised-diff lines** at codegen-units 16 and 1, and the kernel symbol appears in neither `.s` —
same-feature calls inline. Behaviour: the plain caller WITHOUT `unsafe` is `E0133` **even under
`-C target-cpu=x86-64-v3`** and under `-C target-feature=+avx2`, so the attribute on the CALLER is
what discharges it, not the build flag. Also probed: `clippy::unnecessary_safety_doc` does NOT
fire on a safe `#[target_feature]` fn carrying a `# Safety` section — those docs in `sdf_simd.rs`
are correct as written.

**Exceptions.**
- The island is bounded by ERG-15: a `#[target_feature]` fn **cannot** be coerced to a safe `fn`
  pointer (`E0308`) or passed to an `Fn` bound (`E0277`) — verified, EV-77 — so a shell/inner
  split keeps its featured half concrete and its generic half outside the island.
- The boundary sentence must name the fact that is actually true HERE. On this checkout that fact
  is the crate-wide `cfg(target_feature = "avx2")` under the `x86-64-v3` baseline. On a build that
  ever ships a runtime-dispatched arm it would be an `is_x86_feature_detected!` result instead,
  and the sentence changes with it.
- Pointer intrinsics keep their `unsafe` and their own SAFETY clause; deleting THOSE is not what
  this rule licenses.
- A `#[target_feature]` fn is not `const` and cannot be used in a `const` context.

---

### ERG-48 — An obligation that NO REPRESENTATION can hold is carried by a BOUND or by a returned VALUE, never by a sentence repeated at every site

**Binding:** MUST for shapes 1–3 where the site exists; MAY for shape 4 · **Cost:** ZERO-COST
(EV-96, EV-97, EV-98, EV-99, EV-108) · **Guarantee level:** language (a sealed marker, an empty `unsafe`
trait and a consuming call are all resolved by trait selection and borrowck); measured (codegen)

**Rule.** Every other rule in this guide puts a fact into a value's REPRESENTATION — a niche
(ERG-04), a width (ERG-02), a marker row (ERG-05), a bound (ERG-03, ERG-31), a lifetime (ERG-06),
a layout (ERG-01). That axis is finished, and it is why a source-cut survey now returns mostly
confirmations. What the guide had almost no vocabulary for is the other family: an invariant that
is a property of a value STREAM, of a BUFFER'S HISTORY, or of an EXTERNAL AGENT'S TIMELINE, which
no representation can hold and which therefore lives in prose and is repeated. The cost of the gap
is measurable in this tree: **118 "fence-waited" preconditions across 23 RHI files**, **10 of
`colored.rs`'s 50 `// SAFETY:` blocks restating one coloring invariant** plus the two `unsafe
impl`s whose whole justification it is, and a named `CQ-PACK1` invariant written out four times in
`command_queue.rs`. Rust has three tools for exactly this, and all three are free.

**Shape 1 — a REGIME the pointer's bits cannot express is a sealed TYPE PARAMETER.** The tree has
two alignment regimes distinguished only by which intrinsic the author typed: aligned `read` /
`write` off `ComponentPool` / `VmColumn` row bases, and `read_unaligned` / `write_unaligned` in the
byte-packed command queue, the event erased buffer and the log lane — **125 occurrences across 20
files**. ERG-22 governs the FORM of a pointer and its own exception says `NonNull` "conveys
non-nullness ONLY — nothing about validity, alignment or aliasing"; ERG-05 governs the marker row.
Neither makes alignment a type-level fact, so an unaligned buffer and an aligned one have the SAME
TYPE and the wrong `read::<T>()` compiles.

```rust
mod sealed { pub trait Sealed {} }
pub trait IsAligned: sealed::Sealed {
    /// # Safety: `p` points to an initialised `T`; alignment is what this marker permits.
    unsafe fn read_ptr<T>(p: *const T) -> T;
}
pub struct Aligned;   impl IsAligned for Aligned   { #[inline] unsafe fn read_ptr<T>(p: *const T) -> T { unsafe { p.read() } } }
pub struct Unaligned; impl IsAligned for Unaligned { #[inline] unsafe fn read_ptr<T>(p: *const T) -> T { unsafe { p.read_unaligned() } } }

#[repr(transparent)]
pub struct RowPtr<'a, A: IsAligned = Aligned>(NonNull<u8>, PhantomData<(&'a u8, A)>);
impl<'a> RowPtr<'a, Aligned> {
    /// One-way downgrade; there is no `to_aligned`.
    #[inline] pub fn to_unaligned(self) -> RowPtr<'a, Unaligned> { RowPtr(self.0, PhantomData) }
}
```

*Before* — `crates/boyko_ecs/src/ecs/core/commands/command_queue.rs`, whose header names the
invariant in prose ("invariant CQ-PACK1: no `&` / `&mut` reference creation into the byte slots")
and repeats it in three doc blocks and inside the SAFETY comment;
`ecs/core/events/erased_buffer.rs`; `crates/boyko_log/src/lane.rs` — against
`ecs/memory/component_pool.rs`, `vm_column.rs` and `archetype.rs`'s `Column`, the aligned regime,
whose row pointers are the same `*mut u8`.
*After* — `RowPtr<'_, Unaligned>` on the packed side, `RowPtr<'_, Aligned>` on the column side;
CQ-PACK1's four prose copies become one type.
*Verified* — EV-96: **73 = 73, 60 = 60 and 150 = 150 instructions with 0 normalised diff lines** in
three pairs at codegen-units 16 and 1; `RowPtr` is 8 bytes, both markers are ZSTs; the transposition
is `E0308`; a downstream `impl IsAligned` is `E0277` on the seal (ERG-20 clause 5).

**Shape 2 — a property of an INDEX STREAM is an empty sealed `unsafe` MARKER TRAIT.** ERG-07's own
Exceptions say it outright — "the split does not prove index DISJOINTNESS; the colouring does.
`row_ptr`'s `# Safety` says so" — and then stop, leaving the load-bearing fact in prose while the
tree restates it at scale: `ScratchSolveView::row_ptr` carries "the caller guarantees no other
worker writes the same `index` concurrently (the coloring distinct-index invariant)", **84
`row_ptr(` call sites across 19 files**, and the ten `colored.rs` blocks above. No lifetime and no
representation can hold "these indices are pairwise distinct" — an empty `unsafe` trait is exactly
what that is for, and ERG-20 clause 5 supplies the seal.

```rust
/// # Safety
/// Writing element `i` through this view must not observably touch element `j != i`, and the
/// caller's index stream must be pairwise distinct within one parallel step.
pub unsafe trait DistinctIndexWrites: sealed::Sealed {}
// SAFETY: rows are `size_of::<T>()` apart and never overlap; the colouring supplies distinctness.
unsafe impl<T> DistinctIndexWrites for ScratchSolveView<'_, T> {}

fn solve_color<V: DistinctIndexWrites + RowWrite>(view: &V, span: Range<usize>) { /* the obligation IS the bound */ }
```

*Before* — `crates/boyko_physics/src/solver/colored.rs`: `solve_color`, `solve_color_dispatch`,
`solve_color_avx2`, the struct-level soundness note and the `unsafe impl Send` / `Sync` beneath it,
whose entire justification is the C2 coloring invariant.
*After* — the bound; each of the ten SAFETY paragraphs loses its disjointness clause and keeps its
in-bounds clause, which is the split ERG-20 clause 6 asks for.
*Verified* — EV-97: **ICF alias `solve_prose = solve_bound` at codegen-units 16 AND 1** — one
symbol; the cohort gather is 43 = 43 with 0 diff lines; a type without the impl is `E0277`, a
downstream `unsafe impl` is `E0277` on the seal.

**Shape 3 — a property of an EXTERNAL AGENT'S TIMELINE means ownership returns through a CALL.**
Rust ownership does not model "the GPU is still writing here", and `Drop` cannot enforce it because
any value can be leaked safely. In this tree the fact is entirely prose: **118 occurrences of
"fence-waited" / `wait_idle` across 23 files**, nearly all as a `# Safety` precondition of a
`destroy`. ERG-06 governs a dangerous BORROW; ERG-43 governs an UNDO; ERG-11 and REF-04 already
record that `#[must_use]` is not enforcement. None of the three says "ownership returns through a
call".

```rust
#[must_use = "the submission owns these bytes until the fence signals"]
pub struct Pending<T> { fence: FenceId, payload: ManuallyDrop<T> }
impl Device {
    pub fn submit<T>(&self, t: T) -> Pending<T> { /* ownership leaves Rust here */ }
    /// The ONLY way back: consumes the token, so no path can destroy `T` earlier.
    pub fn reclaim<T>(&self, p: Pending<T>) -> T { self.wait_fence(p.fence); /* … */ }
}
```

*Before* — `crates/boyko_rhi_vulkan/src/accel.rs`'s `destroy_accel`: "no submission building /
tracing it is pending (caller: fence-waited or `wait_idle`'d), and it is destroyed once
(by-value)"; the same sentence across `accel_build.rs`, `bindless.rs`, `brick_atlas.rs`,
`ddgi.rs`, `compute.rs`.
*After* — `submit` / `reclaim`; `destroy` becomes unreachable without the token, and the price is
small — `destroy_accel` has four direct call sites plus one through the RHI trait.
*Verified* — EV-98: **14 = 14 instructions, identical opcode histograms**, six diff lines all
register-name choice; `Pending<Accel>` is 32 bytes; dropping the token is a `#[must_use]` error
under `-D warnings`.
⚠️ *The honest falsifier, and the reason the rule is written structurally* — `mem::forget(pending)`
STILL COMPILES (EV-98). The payload is therefore `ManuallyDrop` and the device owns the bytes: a
leak leaves the resource alive, which is a leak and not a use-after-free. A `Drop` tripwire would
not have bought this, and this rule does not pretend otherwise.

*Clause added by the fourth sweep — the payload is `'static`, and that is what makes the leak
harmless.* The sentence above is only true while the payload is not a BORROW. `submit(&mut local[..])`
followed by `mem::forget` compiles today, and what it leaves behind is a device writing into a
frame that has been reclaimed — the falsifier at its worst. The standard fix comes from the
community that met this first, the Embedonomicon's DMA chapter, and it is not a wrapper but a
BOUND: `pub fn submit<T: 'static>(&self, t: T) -> Pending<T>`. MEASURED (EV-108): the bounded and
unbounded spellings are **`drive_b = drive_a`, an ICF alias at codegen-units 16 AND 1** (15
instructions — a bound emits no code), and the hazard above goes from compiling clean to
`error[E0597]: local does not live long enough`. The bound does not stop `mem::forget`; it stops
the payload from being a borrow, so a leaked token leaves a LIVE resource rather than a dangling
one, which is the difference between a leak and UB. Where the payload genuinely cannot be
`'static` — a scoped upload staged from a frame — the answer is the scoped-closure half of ERG-06,
not a longer SAFETY paragraph. (This engine's own uploads go through `HostVisibleBlock`'s
persistently-mapped allocation, which outlives every submission, so the bound is satisfiable for
free at the sites that motivated shape 3.)

**Shape 4 (MAY) — a "must give it back" is a `#[must_use]` linear TICKET whose `Drop` tripwire is
`#[cfg(debug_assertions)]` only.** ERG-43's Exceptions name the hole and leave it open: "the window
between take and put-back is a state where the field is EMPTY and observable". There are 85
`mem::take` / `mem::replace` sites here, including the P19 Tree-Borrows fix in `command_queue.rs`
and the UI layout scratch protocol, and nothing in the type system says the value must come back.
ERG-43 also forbids an always-on panicking drop — a drop that panics while unwinding aborts — which
is precisely why the tripwire is cfg'd, the same shape ERG-26 clause 5's `MaybeDbg` blesses.

```rust
#[must_use = "the row was taken out and must be returned with put_back"]
pub struct Ticket<T> { index: u32, _m: PhantomData<fn() -> T> }
#[cfg(debug_assertions)]
impl<T> Drop for Ticket<T> {
    #[cold] fn drop(&mut self) { panic!("invariant: row {} taken and never returned", self.index) }
}
// take_reserve(i) -> (Ticket<T>, T) · put_back(t, v) consumes · forget_ticket(t) is the explicit discard
```

*Verified, and this is what decides the shape* — EV-99, in RELEASE with a panicking operation
BETWEEN take and put-back: the bare `mem::take` pair is **24 instructions with no `.seh_handler`
and no personality reference**; the cfg'd-`Drop` ticket is **24 with none either** (2 diff lines,
both the panic-location constant); the ALWAYS-on `Drop` is **37 instructions, 4 calls, 3 `ud2`,
two `.seh_handler` and one `rust_eh_personality` — a real landing pad**. `Ticket<T>` is 4 bytes in
BOTH profiles. In debug, dropping a live ticket panics with the message; `forget_ticket` is
silent; `let _ = take_reserve(..)` is a `#[must_use]` error under `-D warnings`.

**What it buys.** An obligation stated once in a bound cannot rot out of step with the code, and a
reviewer counting SAFETY clauses stops seeing a sentence that is true at 39 sites and false at the
40th. Each shape also turns a whole-class review into a compiler error: the wrong `read::<T>()`,
the un-coloured view, the early `destroy`, the un-returned row.

**Exceptions.**
- Shapes 1–3 are MUSTs only where the site exists. Do not mint a marker for a property that has
  ONE holder and one user — that is REF-30, a trait with one implementor.
- Shape 2's marker states DISJOINTNESS and nothing else; the in-bounds fact stays a SAFETY clause
  with its own upholder (ERG-20 clause 6). A marker that tries to state both states neither.
- Shape 3 does not survive `mem::forget`, and no Rust type does. It is structural — the payload is
  `ManuallyDrop`, the device owns it, and the `'static` bound (clause above, EV-108) makes what
  survives a leak rather than a dangling write — not a tripwire; do not sell it as leak-proof.
- Shape 4 is a MAY, and it stays a MAY: `Option<Ticket<T>>` is 8 bytes (no niche in a `u32`), so a
  ticket that must be optional wants a `NonMaxU32` index (ERG-04 clause 5) before it is stored.
- The `#[cfg(debug_assertions)]` `Drop` must not change the type's SIZE; gate both profiles
  (ERG-01's two-sizes exception). It is 4 bytes in both here, measured.
- An always-on panicking `Drop` is refused by ERG-43 and is the arm EV-99 priced; if a release
  tripwire is genuinely wanted, it aborts rather than panics, and that is a different decision with
  its own site.
