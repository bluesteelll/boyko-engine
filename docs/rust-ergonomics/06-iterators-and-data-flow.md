# §6 Iterators and data flow — ERG-35 · 36 · 38

Part of [RUST-ERGONOMICS.md](../RUST-ERGONOMICS.md). Cost verdicts cite [EVIDENCE.md](EVIDENCE.md).
Code is cited by file path and item name, never by line number.

The theme: the loop is where the frame's time goes. Write it in the form that hands LLVM the most
facts — one slice length instead of two, an exact count instead of a guess, a `ControlFlow` token
instead of a flag — and verify identity with the index loop AT THE SITE, because the ledger has one
adaptor that is smaller and slower (`chain`).

---

### ERG-35 — Iterator adaptors are the default loop form; `zip` for paired columns; `filter_map` / `chunks_exact`; `chain` never inside a per-element loop; identity with the index loop is verified per site

**Binding:** SHOULD (adaptors, `zip`, `filter_map`, `chunks_exact`); MUST-NOT (`chain` per
element) · **Cost:** ZERO-COST where verified (EV-20, EV-51, EV-60); `chain` COSTS — 1.7× on an
L1-resident loop, 3–8 % at column scale (EV-21, EV-60) · **Guarantee level:** measured per site —
identity is usual, not guaranteed

**Rule.** Two columns walked together are `a.iter().zip(b)`, not an index loop over both;
`filter_map` replaces `filter().map()`; `chunks_exact(n)` (with `.remainder()` handled) replaces
`chunks(n)`; two sequences walked in turn are two loops. A claim that an adaptor chain is
identical to the index loop in a hot ECS loop is settled with `--emit=asm` at the site — built
with `-C codegen-units=1` if the ICF alias is the evidence wanted, because the shipped release
profile is codegen-units 16 and folds only within a unit.

**Before / After** —

```rust
for i in 0..a.len() { s += a[i] * b[i]; }             // a second bounds check + a panic block for b[i]
for (&x, &y) in a.iter().zip(b) { s += x * y; }        // == the hand-hoisted `let n = a.len().min(b.len())` form
```

The 28 `.chain(` sites under `crates/*/src` are bind-group construction, validation and a
per-LIGHT walk — none per element; keep it that way.

**What it buys.** `zip` gives LLVM one length instead of two, so the `panic_bounds_check` path
disappears and — on 64-byte components — the vector-op count nearly doubles; `chain` builds a
two-state iterator whose state test survives into the loop.

**Verified.** EV-51: index loop 67 instructions with a panic block; `zip` 61, 0 diff lines against
the hand-hoisted form, same time. EV-60: on 64-byte components, index 80 instructions / 23 vector
ops / 2 panic sites vs `zip` 69 / 39 / 0; the hoisted-release-assert form 76 / 40 / 1 cold site and
still the fastest of the three. EV-21: `chain` is 68 instructions vs 87 for two loops and ~1.7×
SLOWER on an L1-resident loop — the case where reading asm alone gives the wrong verdict; EV-60:
3–8 % at column scale, direction preserved in every run.

**Exceptions.**
- **`zip` truncates silently where indexing panics** — by ERG-26 case 2 a silent partial update IS
  silent corruption. For caller-supplied slices: a hoisted RELEASE `assert!(a.len() == b.len(),
  "invariant: …")` above the loop (the `assert!` form, not `assert_eq!`); `debug_assert_eq!` only
  for archetype-paired columns whose equal length the surrounding code establishes.
- `f32` reductions do not vectorise through any adaptor (addition is not associative); a
  reduction that must vectorise is written lane-wise as `boyko_physics/src/solver/simd.rs` does,
  under `boyko_math`'s bit-determinism rule (ERG-39).
- "Hoist config into locals" is a readability choice, not a rule (REF-21); `OnceLock::get()` per
  element is refused (REF-28).

---

### ERG-36 — Iterator contracts are honest and typed: `size_hint` exact only when free and `(0, None)` over a guess; a release check under any `unsafe` write sized by `len()`; `IntoIterator for &T` / `&mut T` with the read-only bound as the gate; `Extend` on reserved capacity; never `FromIterator`

**Binding:** MUST (`(0, None)` over a guess; the release check; the `ReadOnlyQueryData` bound on
`IntoIterator for &Query`; `FromIterator` on VM-backed storage is MUST-NOT); SHOULD (an exact
hint where it is O(1) — the beneficiary is `collect` on the boot / tools / test layers, not the
kernel; the `IntoIterator` pair on other collections) · **Cost:** nothing added to `next()`;
`collect` without a hint COSTS 10 reallocations at n = 4096 (EV-46); the desugaring is ZERO-COST
(EV-31) · **Guarantee level:** measured

**Rule.** `size_hint` is either exact or conservative; `ExactSizeIterator` is a contract that a
wrong `len` — safe code — breaks in safe callers, so any frontier write sized from one keeps a
RELEASE assert (ERG-26 case 2). `for x in &q` / `for x in &mut q` must select the access mode by
type: `impl IntoIterator for &Query<D, F> where D: ReadOnlyQueryData` makes `&q` over a
`Query<&mut T>` a type error, forcing `&mut q`. `Extend` appends into capacity the caller reserved
or grows ONCE from an exact length; `FromIterator` — `collect::<Column<_>>()` — would mint a
column from nothing, the `Vec` detour principle 0 forbids.

**Before** — `crates/boyko_utils/src/bit_mask/bit_set.rs`, `BitSetIterator`: `next()` only;
`collect` over a 64-bit set reallocates four times. `for x in q.iter()` where `&q` over
`Query<&mut T>` would silently read.

**After** — `crates/boyko_ecs/src/ecs/core/iters/query/query.rs`:

```rust
impl<'a, 'w, 's, D, F> IntoIterator for &'a Query<'w, 's, D, F>
where D: ReadOnlyQueryData, F: QueryFilter   // `&q` over a `Query<&mut T, _>` is a type error
{ type Item = D::Item<'a>; type IntoIter = QueryIter<'a, 's, D, F>; fn into_iter(self) -> Self::IntoIter { self.iter() } }
```

`crates/boyko_ecs/src/ecs/memory/vm_column.rs`, `extend_exact` (grow once from `len()`, release
check per element, the `Lying<I>` test); `iters/query/iter.rs`, `QueryIter::size_hint` returning
`(0, None)` with the reason at the site. For `BitSetIterator` — a SKETCH against its real fields
(`bitset`, `next_index`), not compiled against the tree: the popcount must be guarded, because
shifting by `T::BITS` is the overflow `next()` itself guards against:

```rust
fn size_hint(&self) -> (usize, Option<usize>) {
    let n = if self.next_index >= T::BITS { 0 } else { (self.bitset.value() >> self.next_index).count_ones() as usize };
    (n, Some(n))
}
```

**What it buys.** An exact hint makes `collect` allocate once; a wrong exact hint is worse than
none. The bound on the `&Query` impl is free strength — an access-mode invariant that would
otherwise live in a reviewer's head.

**Verified.** EV-46: `collect` with the default hint at n = 4096 is 1 alloc + 10 reallocs; with an
exact hint 1 + 0 (~1.5×); `extend` into `Vec::with_capacity(n)` is 1 + 0 regardless. EV-31: `for &x
in c` and `for &x in c.iter()` fold to one symbol. Zero `impl FromIterator` exists under
`crates/*/src` today; the rule preserves a property.

**Exceptions.**
- `FusedIterator` only after confirming `next()` keeps returning `None`.
- An `Extend` impl that `reserve`s from `size_hint` inside re-introduces incremental growth.
- On a collection whose only consumer already calls `iter()`, the `IntoIterator` pair states
  nothing new — that is why it is a SHOULD outside the query surface.

---

### ERG-38 — Early exits are `let … else`, let-chains, `matches!` and labelled blocks; a body that must `continue` or `return` through a closure propagates `core::ops::ControlFlow` with `?`, never a `bool` flag

**Binding:** SHOULD (the flattening forms); MUST (`ControlFlow` over a flag in closure-shaped
bodies) · **Cost:** ZERO-COST (EV-33, EV-53, EV-60) · **Guarantee level:** language (syntactic
lowerings); measured

**Rule.** A refutable bind that must diverge is `let Some(x) = … else { return … };`; two
refutable patterns under one condition are an edition-2024 let-chain; a predicate over variants is
`matches!`; a loop body expressed as a closure (an eDSL, a `for_each` callback) that needs
`continue` / `return` semantics returns `ControlFlow` and uses `?` — the std type's `Try` impl is
stable, `#[must_use]`, `no_std`, one byte.

**Before** —

```rust
let mut skip = false;
body(|x| { if cond(x) { skip = true; return; } … });   // a flag every reader must trace
if skip { continue; }
```

**After** — `crates/boyko_threadpool/src/scope.rs`, the steal loop: `if let Some(idx) =
own_local_inj && let Some(t) = drain_one(|| …) { … }`; `iters/query/query.rs`, `Query::get`:
`let Some((arch_ptr, row)) = self.resolve_point(entity) else { … };`;
`crates/boyko_shaderdsl/src/cf.rs`: `pub type Flow = ControlFlow<LoopOp>` with `C::if_(cond, ||
C::cont())?`.

**What it buys.** All four constructs are lowerings to the same control flow; what changes is that
the happy path is readable and a `// SAFETY:` block's stated invariants can be checked by eye.

**Verified.** EV-33: `matches!` vs the three-arm `match` — one symbol. EV-53: `ControlFlow` + `?`
vs a manual `continue` — one symbol; `size_of::<ControlFlow<()>>() == 1`. EV-60 stress case: a body
closure capturing TWO mutable locals through an `#[inline(never)]` generic driver — `ControlFlow`
and the `bool` flag folded to one symbol.

**Exceptions.**
- The 2024 `if let` scrutinee temporary lives through the THEN block (only the drop point before
  `else` moved). A D5 reference minted in a scrutinee whose then-block runs a task body is bound
  in its own statement first — `docs/threadpool/KE16-DESIGN-A.md` §1.3 / §1.7 own that shape and
  distinguish the two permitted `worker.rs` forms; this guide only points there.
- A labelled block cannot `continue`; when the block is long, principle 3 prefers an extracted `fn`.
- `ControlFlow`'s variant names are inverted relative to a loop's `continue`; wrap them in named
  combinators (`Cf::cont`), do not spell `Break(Continue)` at call sites.
