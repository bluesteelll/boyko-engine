# §5 Const evaluation and `cfg` — ERG-29 · 31 · 33

Part of [RUST-ERGONOMICS.md](../RUST-ERGONOMICS.md). Cost verdicts cite [EVIDENCE.md](EVIDENCE.md).
Code is cited by file path and item name, never by line number.

The theme: work the compiler does before codegen is work the frame never does. A `const`, an
associated `const`, an inline `const { }` block, a `compile_error!` and a `macro_rules!`
expansion all finish before the first instruction is selected. The rules below make sure the
const evaluator SEES the decision — and one of them (ERG-31) makes a refactor that would hide it
a compile error.

---

### ERG-29 — What can be `const` is `const`: a POD config's defaults are `const DEFAULT: Self`; constructors and accessors of `Copy` value types are `const fn` taking `self`; never `..Default::default()` on an allocating `Default`

**Binding:** MUST (`const DEFAULT` for POD config; MUST-NOT the allocating functional update);
SHOULD (`const fn` + by-value `self` on `Copy` value types) · **Cost:** identical for POD (EV-28,
EV-50); the allocating base COSTS one malloc + one free per construction (EV-45); a wide by-value
receiver is passed by pointer with NO copy (EV-60) · **Guarantee level:** measured

**Rule.** `pub const DEFAULT: Self = Self { … };` beside a plain-data struct, `Default` delegating
to it, call sites writing `Cfg { substeps: 2, ..Cfg::DEFAULT }`. `pub const fn new(…) -> Self`
and `pub const fn index(self) -> usize` on ids, slots, flags, codes and math — `const` is what
admits the value to an ERG-01 gate and an ERG-31 call-site constant; `self` on `Copy` matches the
house macro. For a struct whose `Default` builds a `String` / `Vec` / `Box`, never override a
field through `..Default::default()`: the base is fully constructed and the overridden field's
default is then freed.

**Before** — `let cfg = Cfg { name, ..Default::default() };` where `Default` builds a `String`;
`crates/boyko_utils/src/identifiers/slot.rs`, `Slot`: `pub fn new`, `pub fn index(&self)` — none
`const`, so a `Slot` cannot appear in a gate or a `static`.

**After** — `crates/boyko_scene/src/camera.rs`, `Camera::DEFAULT` with `impl Default` delegating;
`crates/boyko_ecs/src/ecs/identifiers/primitives.rs`, `define_id!`: `pub const fn new(raw) ->
Self` / `pub const fn get(self) -> usize`; `crates/boyko_log/src/codes.rs`, `code_newtype!`'s
`const fn policy(self)` — "the whole reason the rate gate costs nothing at the 74 sites that
declare no damping".

**What it buys.** `..Self::DEFAULT` is a constant with no base construction; a `const fn` accessor
lets the type participate in the tree's dominant idiom (the layout gate) at no codegen change.

**Verified.** EV-28: `..Cfg::DEFAULT` and `..Default::default()` on a POD fold to one symbol.
EV-45 (counting allocator): overriding one field of an allocating `Default` costs 2 alloc + 1
dealloc against 1 + 0 for the literal. EV-50: `const fn index(self)` and `fn index(&self)` fold to
one symbol; the gate `assert!(Slot::new(7, 1).index() == 7)` compiles. EV-60 corrects the previous
draft's mechanism for wide types: a 64-byte `Copy` receiver by value across a non-inlined boundary
is passed INDIRECTLY (a pointer to the caller's value) and is instruction-for-instruction identical
to `&self` — no `memcpy` anywhere. The copy appears only when the callee must mutate or the value
must outlive the call (ERG-08's consuming transition, EV-08).

**Exceptions.**
- Do not contort a body to make it `const`; a function that needs trait calls stays a plain `fn`.
- A `#[non_exhaustive]` struct forbids functional update downstream (ERG-14 clause 4).
- A boot-time `..Default::default()` on a non-POD struct is one malloc at boot and is permitted —
  but if a `DEFAULT` exists, use it.

---

### ERG-31 — A per-type predicate is an associated `const` tested with `if const { T::FLAG }`, so the arm the type does not take is deleted at monomorphisation and a later refactor cannot silently make the condition a runtime load; a per-site policy is bound into a call-site `const`

**Binding:** MUST for kernel generic paths (query data, storage kind, change detection, rate
policy) · **Cost:** ZERO-COST (EV-49, EV-61) · **Guarantee level:** language (an inline `const`
block is evaluated at compile time — RFC 2920); arm deletion is measured compiler behaviour

**Rule.** `trait QueryData { const HAS_DENSE: bool; … }` and, in the generic body, `if const {
D::HAS_DENSE } { … }` — the kernel's own spelling (132 `if const {` lines under `crates/*/src`,
nearly all in `boyko_ecs`). Not a
runtime field on the storage, not a `bool` parameter, not a `match` on a tag read per row, and
not a const-generic parameter (REF-07). Where the predicate comes from a declaration rather than a
type (a diagnostic code's rate policy), a macro binds it into a `const` at the call site so the
same folding applies.

**Before** —

```rust
for row in rows {
    if pool.storage_kind == StorageKind::Dense { /* mask test */ }   // read and branched per row
}
if D::HAS_DENSE { … }   // folds today — but `D::HAS_DENSE` → `self.has_dense` compiles, passes every test,
                        // and reintroduces the per-row load; the previous draft taught this weaker form
```

**After** — `crates/boyko_ecs/src/ecs/core/iters/query/state.rs`: `if const {
Self::HAS_ENABLE_TERM }`, `if const { matches!(E::PROPAGATION, PropagationMode::Down) }`;
`data/mut_.rs`: `const HAS_DENSE: bool = T::STORAGE_IS_DENSE;`; `crates/boyko_log/src/macros.rs`,
`__log_rate_admits!`: `const __BOYKO_RATE: RatePolicy = $Class::policy($code); match __BOYKO_RATE
{ … }` — "the four arms this site does not declare are deleted rather than branched over".

**What it buys.** The instantiation count equals the set of types the kernel already pays for; the
branch and the load it would test leave the loop. And the `const { }` wrapper closes the hole the
plain form leaves: the refactor that turns the predicate into runtime state is a compile error at
the site, not a 0 %-gate regression found by a bench nobody re-ran.

**Verified.** EV-61 (this pass): `if T::HAS_MASK`, `if const { T::HAS_MASK }` and a hand-written
loop with no arm — all three emitted as ONE symbol (`ic_plain_table = ic_const_table`, `ic_none =
ic_const_table`) at codegen-units 1 and at the shipped 16, vectorised; the `= true` instantiations
are 0 normalised-diff lines between the two spellings; `if const { self.flag }` is `error: attempt
to use a non-constant value in a constant`. EV-49: 39 instructions each against the arm-free loop,
plus the placement trap (a 2× wall-clock delta over byte-identical bodies that an unrelated edit
removed).

**Exceptions.**
- A predicate that legitimately depends on runtime state stays a plain `if`; do not contort a body
  to fit `const { }`.
- Dead-arm deletion is compiler behaviour, not a language guarantee: a 0 %-gate claim still needs
  its asm check.
- An associated const with a permissive default (`const SUPPORTED: bool = true`) inherits silently
  into every implementor; the `boyko_rhi` seam tests pin that inherited defaults REFUSE.

---

### ERG-33 — Every illegal `cfg` / feature combination is a `compile_error!` per pair; every conditional site carries the axis banner; a build-variant witness reports the configuration; a test-only substitution is a `cfg`-selected module of `pub use` re-exports

**Binding:** MUST for feature switches and tournament arms; MUST for loom / miri shims ·
**Cost:** compile-time only; a `pub use` re-export is a name binding · **Guarantee level:** language

**Rule.** In `lib.rs`, one `#[cfg(all(feature = "x", feature = "y"))] compile_error!("<axis>: `x`
and `y` are mutually exclusive (<why>)")` per illegal pair. At every `#[cfg]` site of one axis the
same banner comment (`// === KE16 A switch: ke16-a1 / ke16-a1-fifo ===`), so the arm is greppable
and deletable as a unit. One `pub fn variant() -> String` printing the configuration — it allocates,
and that is permitted because it is called once per process at bench or gate entry. A model-checker
substitution is `#[cfg(loom)] pub use loom::sync::atomic::*` / `#[cfg(not(loom))] pub use
core::sync::atomic::*` — never a trait object, never a runtime flag. Custom `cfg`s are registered
in the workspace `unexpected_cfgs` table.

**Before** — two features that silently combine and produce a number for a configuration nobody
meant to measure; a loom shim behind a `dyn` atomic trait.

**After** — `crates/boyko_threadpool/src/lib.rs`: eleven `compile_error!` arms ("a mis-specified
`--features` line fails to BUILD instead of producing a number …") plus `ke16_variant()`;
`tls.rs`: the banner at each of its three conditional sites; `sync.rs`: the re-exports, with the
list of what is deliberately NOT shimmed and why per item; root `Cargo.toml`: `cfg(loom)` and
`cfg(force_alloc_panic)` registered.

**What it buys.** This repository's recorded failure mode is a gate that could not fail; a feature
combination that compiles and measures the wrong thing is that failure with a number attached.

**Verified.** `compile_error!` and `cfg` resolve before codegen; no runtime row exists or is needed.

**Exceptions.**
- `cfg(miri)` is built in and must NOT be registered in `unexpected_cfgs`.
- A pair Cargo already implies is legal and absent from the list; the list is edited when features
  change and deleted with the tournament.
- An unlisted omission from the shim is a hole; every deliberate non-shim carries its reason.
