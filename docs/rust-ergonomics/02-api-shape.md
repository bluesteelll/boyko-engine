# §2 API shape, naming and documentation — ERG-11 · 12 · 14 · 15 · 17 · 39 · 41

Part of [RUST-ERGONOMICS.md](../RUST-ERGONOMICS.md). Cost verdicts cite [EVIDENCE.md](EVIDENCE.md).
Code is cited by file path and item name, never by line number. (The former §7 — naming and
documentation — is merged here: a name is part of a signature.)

The theme: the signature is where the reader is. It can carry the cost of a call (`to_`
allocates, `&[T]` does not), the contract of a call (a token, an enum instead of a `bool`), and
the safety of a call (`&q` over `Query<&mut T>` is a type error) — all for free.

---

### ERG-11 — `#[must_use]` with a message on every guard, token and builder TYPE, and on every status a caller may not drop

**Binding:** MUST (guard / token / builder types; a `Result` or status newtype whose drop is a
bug — `VkResult`); MAY (pure value methods — only where ignoring the value is a plausible bug,
never by sweeping `clippy::must_use_candidate` across a crate) · **Cost:** ZERO-COST, lint only
(EV-25) · **Guarantee level:** language

**Rule.** The attribute goes on the TYPE for guards, tokens, builders and status newtypes: one
attribute then covers every constructor and setter. The message says what to do, not that
something was unused. The workspace runs clippy with `-D warnings`, so the lint is a build
failure.

**Before** — `crates/boyko_threadpool/src/tls.rs`, `InSystemRunGuard`: no attribute;
`InSystemRunGuard::enter();` compiles, drops on the same line and silently defeats the depth
counter. `crates/boyko_rhi_vulkan/src/ffi.rs`, `VkResult(pub i32)`: no attribute; an FFI call
whose status is never inspected is invisible — ~92 `is_success()` sites, ten of them hand-writing
the `!is_success() && != INCOMPLETE` conjunction of the two-call enumerate idiom. `ThreadPoolBuilder::num_threads(mut self, n) ->
Self`: `builder.num_threads(8);` consumes the builder and loses the setting.

**After** — the house form, `crates/boyko_ecs/src/ecs/core/component/hooks/builder.rs`:

```rust
#[must_use = "the builder commits hooks on drop; bind it or chain a setter"]
pub struct ComponentHooksBuilder<'a> { … }

#[must_use = "bind the guard (`let _g = …`); dropping it immediately ends the system-run window"]
pub struct InSystemRunGuard { _private: () }

#[must_use = "a Vulkan status that is not inspected is an unchecked FFI failure"]
#[repr(transparent)] pub struct VkResult(pub i32);
impl VkResult {
    /// Success, or the `INCOMPLETE` a two-call enumerate legitimately returns.
    #[inline] pub fn ok_or_incomplete(self) -> Result<(), VkResult> { … }   // one name for the ten hand-written tests
}
```

**What it buys.** A misuse that is invisible in a diff becomes a build failure; a discard that is
intentional becomes an explicit `let _ =` with a reason a reviewer can read.

**Verified.** EV-25: the lint emits no code; it fired on the two bare statements in a
10-statement probe. The legibility guard confirmed the TYPE placement covers both `Guard::enter();`
and `B::new().num_threads(8);` with one attribute.

**Exceptions.**
- It is a LINT with holes (REF-04): `let _ = make_guard();` — the actual RAII bug — is silent.
  Never rely on it for soundness; load-bearing invariants are structural (ERG-06, ERG-08) or a
  `Drop` tripwire (ERG-43).
- A reviewer treats `let _ = f();` on a `#[must_use]` value as a claim needing a one-line reason.
- The pure-method half is a MAY because the lint that would decide it fires on essentially every
  pure method — decorating all 49 `boyko_math` `pub fn`s is ceremony (REF-38).

---

### ERG-12 — Return `impl Iterator` (or a named concrete type), never `Box<dyn Iterator>`; name the concrete type for anything the kernel stores

**Binding:** MUST · **Cost:** ZERO-COST for RPIT; `Box<dyn Iterator>` COSTS 1.8–2.8× (EV-13,
EV-60) · **Guarantee level:** language (RPIT is an opaque CONCRETE type)

**Rule.** `-> impl Iterator<Item = T> + '_` for a value consumed at the call site. For a value a
struct or `QueryState` must hold, write the concrete iterator type or a generic parameter — RPIT is
unnameable and the "fix" people reach for is the banned box. Edition 2024 captures ALL in-scope
lifetimes and generics; when the result must NOT borrow `&self`, write `use<…>` naming every
in-scope type and const parameter (omitting one is a compile error, not a looser capture; bare
`use<>` is legal only where none is in scope — verified on this toolchain).

**Before** — `pub fn iter(&self) -> Box<dyn Iterator<Item = (Handle<T>, &T)> + '_>`.

**After** — `crates/boyko_ecs/src/ecs/core/asset/assets.rs`, `Assets::iter`:
`pub fn iter(&self) -> impl Iterator<Item = (Handle<T>, &T)> + '_`.

**What it buys.** `next()` is a static call that inlines; no allocation, no fat pointer.

**Verified.** EV-13: boxed 69 instructions with a real allocator call, 1.8× slower. EV-60: on a
64-byte component column with a box LLVM cannot devirtualise, 2.8× (9 calls per element, 6 vector
ops against 13).

**Exceptions.**
- Auto-traits leak through the opaque type: if the hidden iterator stops being `Send`, downstream
  bounds break with no signature change. Write `+ Send` where it matters.

---

### ERG-14 — A closed set is an exhaustive `#[repr(uN)]` enum: dispatch by tag with no `dyn` or fn-pointer call per element on the kernel or schedule layer; a set of integer constants is an enum at the API surface; `#[non_exhaustive]` only on the External list

**Binding:** MUST · **Cost:** enum tag ZERO-COST; `dyn` / fn-ptr COST 2.5–5× on throughput loops
(EV-14, EV-60); `as uN` on a `#[repr(uN)]` enum is a no-op (EV-59) · **Guarantee level:** measured

**Rule.** Four clauses:
1. A build-time classification (system kind, storage kind, cloneability) is a `#[repr(u8)]` enum
   with a `const fn` predicate over `matches!`. Hoisting the `match` out of the per-element loop
   is a READABILITY choice — LLVM unswitches the inner form to the same code (EV-60).
2. On the kernel and schedule layers there is no `dyn` and no fn-pointer call per ELEMENT. Erasure
   amortised per system, task or panic is accepted, and the accepted sites are the LIST in the
   index (`Box<dyn System>`, `BoolSystem`, `TaskHandle::body`, `Box<ScopeShared>`, the cold
   `panic_payload`). A `Box<dyn>` on those rows that is not in the list is reported, not
   registered — "per element" cannot be written in the amortisation column.
3. A type-erased per-TYPE operation selected by an id (`CloneFn`, `DropFn`, hook fns) is a bare
   `unsafe fn` pointer table with a trivially-copyable bypass, called per element on the
   lifecycle layer only — the erased body is user code that must run regardless.
4. A closed set of integer constants — `sdf_op::{UNION, SUBTRACT, INTERSECT}`, a diagnostic
   class byte, a fence state — is a `#[repr(u32)]` / `#[repr(u8)]` enum at the API surface. The
   GPU-facing or wire FIELD stays the integer; the constructor and the dispatch take the enum,
   so `SdfEdit::sphere(c, r, 7, 0.0)` (today: compiles, silently falls through to `UNION`) is
   a compile error and adding a fourth op makes every `match` red instead of silently defaulting.
   `#[non_exhaustive]` goes only on a type in the External row of the layer table (the published
   error enums and `KeyCode`) — inside the workspace it turns "add a variant → N compile errors"
   into "N silent `_ =>` fallthroughs" (EV-26).

**Before** — `crates/boyko_sdf_math/src/lib.rs`: `pub mod sdf_op { pub const UNION: u32 = 0; …}`,
`pub fn sphere(.., op: u32, ..)`, `combine(.., op: u32, ..)` as a chain of `if op == ..` with a
silent else. `crates/boyko_log/src/codes.rs`: a module doc titled "Class is a TYPE, not a field"
whose own lookup seam then takes `class: u8` — `explain(b'w', 2207)` returns `None` forever, and a
`u8` next to a `u16` is exactly the pair that gets transposed.

**After** — `crates/boyko_ecs/src/ecs/core/component/component_registry/clone.rs` (`Cloneability`,
`CloneFn`, the "0%-gate (grep-proof obligation)" line); `schedule/system_box.rs`, `SystemKind`
(a 1-byte tag in the slot a `bool` occupied, read at ONE dispatcher branch); and

```rust
#[repr(u32)] #[derive(Clone, Copy, PartialEq, Eq)] pub enum SdfOp { Union = 0, Subtract = 1, Intersect = 2 }
pub fn sphere(center: [f32; 3], radius: f32, op: SdfOp, smoothness: f32) -> Self { .. op: op as u32 .. }
#[repr(u8)] pub enum CodeClass { Warn = b'W', Error = b'E', Panic = b'B' }
const _: () = assert!(CodeClass::Warn as u8 == b'W');
```

**What it buys.** The tag inlines to a branch tree LLVM hoists and vectorises across; an indirect
call blocks vectorisation. An integer that could be any of 2^32 values becomes one of three the
compiler enumerates, and exhaustiveness — the best refactoring tool this codebase has — is kept.

**Verified.** EV-14: throughput loop — tag 0.233 ns/elem = monomorphic; `&dyn` 1.17; fn ptr 1.18
(5×); latency-bound loop — indistinguishable (state the loop shape with every dispatch claim).
EV-60: `dyn` per element on a 64-byte component 2.5–3×, 2 vector ops against 88; `match` inside
vs outside the loop 0.536 vs 0.540 ns. EV-59: `mk_r(SdfOp)` writing `op as u32` and `mk_c(u32)`
emitted identical 3-instruction bodies; `combine` as a `match` and as `if op == const` chains
emitted the same opcode multiset with the enum form one byte shorter. EV-26: `#[non_exhaustive]`
from a sibling crate — `E0004`, `E0638`, `E0639`; the `as`-cast folklore is wrong (only a marked
VARIANT blocks it).

**Exceptions.**
- An enum is as large as its largest variant; past ~4 arms with payloads, measure a jump table
  against a well-predicted indirect call.
- The eDSL body is instantiated over both `f32` and `Emit`; a `match` over `SdfOp` must lower
  identically on both — re-run the `*_edsl_sync` and `cpu_gpu_sdf_agreement` gates.
- `enum_dispatch` / `bitflags` crates for fifteen lines of `match` are REF-10.

---

### ERG-15 — Kernel signatures are concrete (`&[T]`, `&mut [T]`, `&str`, `&Path`); conversion sugar lives at the boot boundary, as a thin generic shell over one concrete inner `fn`

**Binding:** MUST (concrete kernel signatures); SHOULD (the shell/inner split at boot) · **Cost:**
`impl Into<String>` COSTS an allocation by definition and monomorphises per argument type (EV-18);
`&[T]` costs nothing · **Guarantee level:** definitional + measured

**Rule.** On the kernel layer a function takes the engine's own storage by reference. `impl
Into<String>`, `impl AsRef<Path>`, `impl IntoIterator` are permitted on the boot layer only, and
there the generic signature is a shell: `fn inner(path: &Path) -> R { /* the work, once */ }
inner(arg.as_ref())`. A kernel entry that must take an iterator takes `impl ExactSizeIterator<Item
= T>` and keeps a release check per element because `len()` is safe code that can lie (ERG-36).

**Before** — `pub fn load_shader(path: impl AsRef<Path>) -> Result<..> { /* 200 lines, copied per caller type */ }`.

**After** — `crates/boyko_ecs/src/ecs/memory/vm_column.rs`, `extend_exact(&mut self, iter: impl
ExactSizeIterator<Item = T>)` (kernel); `ThreadPoolBuilder::thread_name_prefix(impl Into<String>)`
(boot — the tree's four `impl Into<String>` signatures are all on the boot row); and the shell:

```rust
pub fn load_shader(path: impl AsRef<Path>) -> Result<ShaderModule, LoadError> {
    fn inner(path: &Path) -> Result<ShaderModule, LoadError> { /* the real work, once */ }
    inner(path.as_ref())
}
```

**What it buys.** `&[T]` at a kernel boundary is deliberately LESS flexible: the caller must have
materialised the data in the engine's own storage (principle 0). The shell keeps the convenience
at the signature and one copy of the cold body (principle 3's I-cache half).

**Verified.** EV-18: at 32 instantiating types, 2132 vs 614 instruction lines (3.5×) and +25 %
cold build; the shell costs one instruction at n = 1. EV-12: `impl Trait` in argument position and a
named generic fold to one symbol. EV-39 is recorded honestly: the allocation probe could not show
the `String` because LLVM deleted a dead one; the rule stands on the definition of `Into<String>`.

**Exceptions.**
- **Principles 2/6 win on hot generic kernels:** `Query::for_each`, the SIMD solver loops,
  `Bundle` materialisation — there the monomorphisation IS the optimisation. Never de-specialise.
- `AsRef<Path>` accepts `String`; prefer `&Path` even at the boundary when the caller has one.

---

### ERG-17 — `Deref` only from a buffer to its slice, or from a smart pointer / guard to the single value it owns; never from a domain newtype, a proof-carrying type or a solve view; a `Deref` with a side effect says so at the impl

**Binding:** MUST-NOT (newtype / proof / solve view); the two permitted forms carry their cost in
the doc · **Cost:** the slice form is ZERO-COST (EV-16); the newtype form COSTS the
encapsulation (EV-17) · **Guarantee level:** measured

**Rule.** Three cases:
1. `Deref<Target = [T]>` on a type that IS a buffer (`InlandStore`, a scratch column): permitted.
2. `Deref<Target = T>` on a wrapper whose target IS the single value it owns — the system params
   `Res`, `ResMut`, `NonSendRes`, `Local`, `State`, and the change-tracked `Ref<T>` / `Mut<T>`:
   permitted, and the house idiom. When the deref does WORK — `Mut::deref_mut` sets a flag and
   touches the change tick under a SAFETY block — the impl's doc says so, because `*m = v` shows
   the reader nothing.
3. `Deref<Target = u32>` on an id, `Deref<Target = Inner>` to "inherit" methods, any `Deref` on
   a proof-carrying type (ERG-03) or on a solve view (ERG-07): forbidden.

**Before** — the two-line impl that deletes a guarantee:

```rust
impl Deref for RegisteredComponentId { type Target = u32; fn deref(&self) -> &u32 { &self.0 } }
```

**After** — `crates/boyko_ecs/src/ecs/core/entity/inland_store.rs`, `impl Deref for InlandStore
{ type Target = [EntityInland]; … }` (case 1); `crates/boyko_ecs/src/ecs/core/iters/query/data/mut_.rs`,
`impl DerefMut for Mut<'_, T>` with its STORE3 / MUT3 SAFETY (case 2).

**What it buys.** `Deref` participates in method resolution and re-exposes every inner method by
auto-deref. On a solve view a `DerefMut<Target = [T]>` would hand every worker the whole-buffer
`&mut [T]` ERG-07 exists to make un-typeable — the O11-SP4 race reintroduced by two lines no test
would fail.

**Verified.** EV-16: `get(i)` through `Deref<[u64]>` and through `as_slice()` folded to one
symbol. EV-17: a two-line `Deref<Target = u64>` made `count_ones`, `leading_zeros`, `wrapping_mul`
and `*d` compile from a downstream crate.

**Exceptions.**
- An inherent method on a `Deref` type that shadows a slice method silently changes every call
  site; name it something else.

---

### ERG-39 — Names and operators say what a call costs and means: `as_` / `to_` / `into_`, no `get_` getter, one name per operation; a positional `bool` the method name does not carry is a two-variant enum; a math or flag type implements the full operator set

**Binding:** MUST (the naming conventions; the `bool` → enum swap where the method name does not
carry the parameter's noun); SHOULD (the operator set, with the bit-determinism exception) ·
**Cost:** names emit nothing; the enum is ZERO-COST or one instruction shorter (EV-42, EV-59,
EV-60); operators are ZERO-COST (EV-30, EV-43) · **Guarantee level:** measured

**Rule.**
- `as_x(&self) -> &X` is a free view; `to_x(&self) -> X` on a non-`Copy` receiver allocates and
  belongs off the hot path; `into_x(self) -> X` consumes. A getter is `fn frame(&self)`; `get`
  is the fallible form. `clear(&mut self)` empties the container.
- A positional `bool` is an enum when the method name does not contain the parameter's noun — the
  mechanical decider. `pin_workers(on)` and `with_hot_reload(true)` keep the `bool`;
  `create_fence(true)`, `match_word(p, w, !want)`, `set_enable_bit(tag, row, true) -> bool` do not.
- A `Copy` math aggregate implements `Add`/`Sub`/`Mul<f32>`/`Div<f32>`/`Neg`, all four `*Assign`
  forms and `Mul<VecN> for f32`; a flag newtype implements `BitOr` AND `BitOrAssign`. `#[inline]`,
  never `#[inline(always)]`.

**Before** — `crates/boyko_ecs/src/ecs/core/iters/query/enable_terms.rs`: polarity travels as
`with: bool`, a bitmask, `want`, and reaches `enable_store.rs`'s `match_word(page, word, invert:
bool)` as `(*col).match_word(p, w, !want)` — two negations to resolve, and a stray `!` is a
SILENT WRONG ANSWER (the complementary row set, no panic). `archetype.rs`,
`set_enable_bit(tag, row, value: bool) -> bool`: the incoming `bool` is set/clear, the outgoing
`bool` is "allocated the column for the first time", and four in-file call sites drop the return.
`crates/boyko_rhi/src/device.rs`, `create_fence(&self, signaled: bool)`.
`crates/boyko_physics/src/solver/colored.rs`, `solve_all_colors(.., bias_active: bool,
parallel: bool, simd: bool)`: three positional `bool`s in a row whose nouns are not in the method
name — the decider fires on all three. `crates/boyko_rhi/src/enums.rs`:
six flag families with `BitOr` and no `BitOrAssign`, so `usage |= X` does not compile while
`usage | X;` compiles and does nothing.

**After** —

```rust
#[derive(Clone, Copy, PartialEq, Eq)] pub enum Polarity { Present, Absent }
impl Polarity { #[inline] const fn inverted(self) -> Self { match self { Self::Present => Self::Absent, Self::Absent => Self::Present } } }
pub(crate) fn match_word(&self, page: PageIdx, word: WordIdx, p: Polarity) -> u64
// call site reads itself:  (*col).match_word(p, w, want.inverted())

enum EnableBit { Set, Clear }
#[must_use] enum ColumnAlloc { FreshlyAllocated, AlreadyPresent }
pub(crate) fn set_enable_bit(&mut self, tag: ComponentId, row: RowIndex, bit: EnableBit) -> ColumnAlloc

#[repr(u32)] pub enum FenceState { Unsignaled = 0, Signaled = 1 }   // to_vk is `state as u32`
impl BitOrAssign for BufferUsage { #[inline] fn bitor_assign(&mut self, r: Self) { self.0 |= r.0; } }
```

(`enable_terms.rs` already has the right OUTER half — named `push_with` / `push_without`
wrappers; the leak is that the inner `bool` travels on unwrapped.)

**What it buys.** The call site reads itself, `.inverted()` becomes the only way to flip a
polarity, and a `#[must_use]` `ColumnAlloc` turns four silently dropped bookkeeping obligations
into build errors. `a |= b` is the spelling every reader knows; `2.0 * v` stops being a type error.

**Verified.** EV-59: `mw_r(u64, Polarity)` and `mw_c(u64, bool)` emitted as a SYMBOL ALIAS;
`size_of::<Polarity>() == 1`, `Option<Polarity>` == 1. EV-42 / EV-60: the two-variant enum was one
instruction SHORTER than the `bool` in a non-inlined callee (the `movzbl` vanishes). EV-30:
operators vs per-field arithmetic — identical opcode histogram; EV-43: `a |= b` = `a = a | b`.
EV-60 refutes the previous draft's reason for `*Assign` ("the optimiser must prove away a
copy-back"): the by-value `*m = *m + k` form was the SMALLER one on a 64-byte matrix. The honest
reason is readability and the missing `2.0 * v`.

**Exceptions.**
- **`boyko_math`'s bit-determinism invariant wins:** `Div<f32>` is `x / s`, never `x * s.recip()`;
  `Neg` is `-x`; no `mul_add`. Rust has no fast-math and `a + b * c` will not contract to an FMA —
  that is the guarantee the shader oracle relies on.
- Do not sweep the `bool` rule across builder setters or named-setter fields
  (`linked(deep)`, `strict(strict)`) — there the enum is ceremony (REF-33).
- `Vec3` is 12 bytes: under `extern "C"` on Win64 it is passed by hidden pointer.

---

### ERG-41 — A `pub` item's doc says why and names the decision; a suppressed check states its reason and retires itself

**Binding:** MUST (`///` on every `pub` item — CLAUDE.md; `#[expect(lint, reason = "…")]` for
any new lint suppression other than the two script-gated forms); SHOULD (`#![warn(missing_docs)]`
per crate, smallest first, raised to `deny` as each goes clean; the two doc sub-forms below) ·
**Cost:** none (EV-55) · **Guarantee level:** language (`#[expect]` stable since 1.81)

**Rule.** Two things a later reader cannot reconstruct and a rewrite loses: the negative-space
list ("deliberately not X, because Y") and the falsified-claim record ("this was documented as
cold; the audit traced the callers and it was not"). Write those; the rest is CLAUDE.md. For
suppressions: `#[expect(dead_code, reason = "…")]` fires `unfulfilled_lint_expectations` the day
the lint stops triggering, so an allow cannot outlive its condition. The two existing gates keep
their spelling — `#[allow(clippy::disallowed_types)]` + rationale + a `docs/HOT-PATH-EXCEPTIONS.md`
row (`scripts/check_hotpath_exceptions.py` greps for it), and `#[ignore = "<class>: …"]`
(`tests/ignore_reasons_census.rs`).

**Before** — `crates/boyko_utils/src/sparse_map/sparse_collection.rs`: `#[allow(dead_code)]` on a
trait whose doc claims an implementor that does not exist. Census: 465 `#[allow(` sites, 3 with a
reason, 1 `#[expect(`. `crates/boyko_utils/src/lib.rs`: four `pub mod` lines and no `//!`; no
crate sets `missing_docs`.

**After** — `crates/boyko_utils/src/type_intern/mod.rs`: names rust#22991 as the forcing
constraint, records that all four prior sites documented themselves "cold, registration-only"
and that the 2026-07 audit found all four claims false, and records the rejected alternative with
its reason. `crates/boyko_threadpool/src/sync.rs`: "Deliberately **not** shimmed:" with a reason
per item. And:

```rust
#[expect(dead_code, reason = "extension point reserved for <rung>; the expectation reds the build the day it gains a use")]
pub trait SparseCollection<K, V>: … { … }
```

**What it buys.** A reason a reader can check is the difference between a control and a comment;
`#[expect]` is the only suppression that reports its own obsolescence.

**Verified.** EV-55: a fulfilled expectation is silent; an unfulfilled one warns with the reason in
the note. Lints emit no code.

**Exceptions.**
- Keep `#[allow]` where the lint fires only under some `cfg` / feature combination, and say so in
  the reason.
- The `missing_docs` first-wave counts and the 462-site `#[expect]` migration are build-and-count
  passes, not keystrokes (OPEN 4).
