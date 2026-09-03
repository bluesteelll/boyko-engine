# §5 Const evaluation and `cfg` — ERG-31 (ERG-29 and ERG-33 both merged into it)

Part of [RUST-ERGONOMICS.md](../RUST-ERGONOMICS.md). Cost verdicts cite [EVIDENCE.md](EVIDENCE.md).
Code is cited by file path and item name, never by line number.

The theme: work the compiler does before codegen is work the frame never does. A `const`, an
associated `const`, an inline `const { }` block, a `compile_error!` and a `macro_rules!`
expansion all finish before the first instruction is selected. The rules below make sure the
const evaluator SEES the decision — and one of them (ERG-31) makes a refactor that would hide it
a compile error.

---

### ERG-29 — merged into ERG-31 (second sweep, 2026-09-02)

*What can be `const` is `const`* is now **ERG-31 clause 1**, unchanged in substance: `const
DEFAULT: Self` on POD config with `Default` delegating (MUST), `const fn` taking `self` on `Copy`
value types (SHOULD), never `..Default::default()` on an allocating `Default` (MUST-NOT), with
EV-28 / EV-45 / EV-50 / EV-60 and all three exceptions. It was merged because ERG-29 and ERG-31
were one rule wearing two ids — both say "make the const evaluator SEE the decision", and ERG-29's
`const fn` accessor exists precisely so the value can appear in an ERG-01 gate and in ERG-31's own
call-site `const`. The merge pays for one of the second sweep's two new rules; see the index's
size paragraph.

---

### ERG-33 — merged into ERG-31 (third sweep, 2026-09-03)

*Every illegal `cfg` / feature combination is a `compile_error!` per pair* is now **ERG-31
clause 4**, carried whole — rule, `Before`, the `boyko_threadpool` `After`, the what-it-buys
paragraph and all three exceptions. It was merged because this section's own title already pairs
them (`Const evaluation and cfg`) and because both are one thesis: make the compiler SEE the
decision, so a configuration nobody meant cannot exist at runtime. ERG-29 was merged into the same
rule on the same reasoning one revision earlier. This merge is the PAYMENT for the third sweep's
one new rule, ERG-48; see the index's size paragraph.

---

### ERG-31 — Compile-time evaluation and configuration: what can be `const` is `const`; a per-type PREDICATE is an associated `const` tested with `if const { T::FLAG }`, so the arm the type does not take is deleted at monomorphisation and a later refactor cannot silently make it a runtime load; a per-type POLICY that must bound a generic is an associated TYPE over sealed markers; every illegal `cfg` / feature pair is a `compile_error!`

**Binding:** MUST for the `const DEFAULT` / allocating-functional-update halves of clause 1, for
kernel generic paths in clauses 2-3 (query data, storage kind, change detection, rate policy) and
for clause 4's feature switches, tournament arms and loom / miri shims; SHOULD for clause 1's
`const fn` accessors · **Cost:** ZERO-COST (EV-28, EV-45, EV-49, EV-50, EV-60, EV-61, EV-80);
COMPILE-TIME ONLY for clause 4 · **Guarantee level:** language (an inline `const` block is evaluated
at compile time — RFC 2920; an associated type is resolved by trait selection); arm deletion is
measured compiler behaviour

**Clause 1 (merged from ERG-29) — what can be `const` is `const`.** MUST: `pub const DEFAULT:
Self = Self { … };` beside a plain-data struct, `Default` delegating to it, call sites writing
`Cfg { substeps: 2, ..Cfg::DEFAULT }`. SHOULD: `pub const fn new(…) -> Self` and `pub const fn
index(self) -> usize` on ids, slots, flags, codes and math — `const` is what admits the value to
an ERG-01 gate and to clause 2's call-site constant, and `self` on `Copy` matches the house macro.
MUST-NOT: `..Default::default()` on a struct whose `Default` builds a `String` / `Vec` / `Box` —
the base is fully constructed and the overridden field's default is then freed.
*Before* — `let cfg = Cfg { name, ..Default::default() };` over an allocating `Default`;
`crates/boyko_utils/src/identifiers/slot.rs`'s `Slot::new` / `index(&self)`, neither `const`, so a
`Slot` cannot appear in a gate or a `static`.
*After* — `crates/boyko_scene/src/camera.rs`'s `Camera::DEFAULT` with `impl Default` delegating;
`define_id!`'s `pub const fn new` / `get`; `crates/boyko_log/src/codes.rs`'s `code_newtype!`
`const fn policy(self)` — "the whole reason the rate gate costs nothing at the 74 sites that
declare no damping".
*Verified* — EV-28: `..Cfg::DEFAULT` and `..Default::default()` fold to one symbol on POD.
EV-45 (counting allocator): overriding one field of an allocating `Default` costs 2 alloc + 1
dealloc against 1 + 0. EV-50: `const fn index(self)` and `fn index(&self)` fold to one symbol.
EV-60 corrects the previous mechanism: a 64-byte `Copy` receiver by value across a non-inlined
boundary is passed INDIRECTLY and is instruction-identical to `&self` — no `memcpy`; the copy
appears only when the callee must mutate or the value must outlive the call (ERG-08, EV-08).
*Exceptions* — do not contort a body to make it `const`; a `#[non_exhaustive]` struct forbids
functional update downstream (ERG-14 clause 4); a boot-time `..Default::default()` on a non-POD
struct is one malloc at boot and is permitted, but if a `DEFAULT` exists, use it.
*Two exceptions added by the fourth sweep, both measured, and both of them limits on the clause
rather than extensions of it.*
- **This clause has a CEILING and it is low.** CTFE is a MIR interpreter, so "compute it at compile
  time" costs interpreted steps. MEASURED (EV-111) on `rustc 1.97.1`: a `const fn` counting loop is
  silent at 100 000 iterations and is `error: constant evaluation is taking a long time` at
  2 000 000 — the `long_running_const_eval` lint, DENY-by-default, hence a hard error out of the
  box. It is a LINT, so `#![allow(long_running_const_eval)]` lets a genuinely long computation
  through on stable — at 20 000 000 iterations that is **1 m 46 s** of compile time (box under
  load) plus five repeated diagnostics that downstream consumers also see. `boyko_image`'s
  `const CRC32_TABLE: [u32; 256] = crc32_table()` is far under the budget and is the right size for
  this clause. A table an order of magnitude larger — an SMAA-shaped LUT, an SDF probe table — is a
  build script or a committed blob with a gate, not a bigger `const fn`; `boyko_render`'s
  `smaa_luts.rs` already ships its two as committed bytes with SHA-256 pins, and that is the
  correct answer, not a failure to apply this clause.
- **The array-repeat inline `const` is NOT a cost change.** `[const { X::new() }; N]` is the
  idiom for an array of a non-`Copy`, const-constructible element, and it reads better than
  `core::array::from_fn(|_| X::new())`. It buys nothing else: MEASURED (EV-110) at the
  `bundle_archetype_cache` shape (`[OnceLock<_>; 1024]`, boxed), the two spellings are **69 = 69
  instructions with byte-identical bodies** at codegen-units 16 and 1 — both materialise the array
  on the STACK with the same vectorised loop and `memcpy` it into the box, exactly the EV-08
  physics. Write whichever reads better; never cite the change as a cost one (REF-21, REF-36).

**Clause 2 — the per-type PREDICATE, as before.** `trait QueryData { const HAS_DENSE: bool; … }` and, in the generic body, `if const {
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

**Clause 3 (added by the second sweep) — a per-type POLICY that must appear in a
where-clause is an associated TYPE over sealed marker ZSTs, not a set of independent `bool`s.**
An associated `const` can be BRANCHED on; it cannot BOUND a generic. Where the kernel needs to
refuse a whole storage class at the SIGNATURE, the policy is a type:

```rust
mod seal { pub trait Seal {} }
pub trait StorageClass: seal::Seal + 'static { const KIND: StorageKind; }
pub struct TableStorage; pub struct BitsetStorage; pub struct DenseStorage;   // + seal impls
pub trait Component: 'static + Sized { type Storage: StorageClass; /* … */ }

fn pool_of<C: Component<Storage = TableStorage>>(m: &EcsMaster) -> &ComponentPool { … }
```

*Before* — `crates/boyko_ecs/src/ecs/core/component/component.rs` carries `const
STORAGE_IS_BITSET: bool = false` and `const STORAGE_IS_DENSE: bool = false` as two INDEPENDENT
consts, so `(true, true)` is representable and is policed by the derive plus runtime `const`
gates in `component_registry/mod.rs` — whose comments name the wrong pairing
(`STORAGE_IS_DENSE` + `RESIDENCY = Gpu`). The runtime enum the markers mirror already exists
(`StorageKind { Table, Bitset, Dense }`), and 35 files under `crates/*/src` reference one or the other.
*What it buys* — the illegal pair stops being representable; the pool-less kinds are refused at
the signature instead of resolving a pool that is not there. That is the monomorphic half of a
recorded defect class (GK-2 / KE11 / KE13: a `ComponentPool` resolved without consulting
`StorageKind` reads NULL as an answer). Clause 2's `if const` arm survives unchanged through
`<C::Storage as StorageClass>::KIND`, and the seal is ERG-20 clause 5's mechanism.
*Verified* — EV-80: **ICF aliases in BOTH instantiations** at codegen-units 1 and 16 —
`if const { C::STORAGE_IS_DENSE }` and
`if const { matches!(<C::Storage as StorageClass>::KIND, StorageKind::Dense) }` compile to one
symbol; every marker is a ZST; `pool_of::<Flagged>` is `E0271`; a downstream `impl StorageClass`
is `E0277`.
*Scope, stated honestly* — this closes the MONOMORPHIC resolvers only. The dynamic
`ComponentId` paths — the "caller who does not resolve at all" of KE13 — stay on the runtime
`StorageKind` enum under ERG-14, and no type can reach them.

**Exceptions (clauses 1–3; clause 4 carries its own).**
- A predicate that legitimately depends on runtime state stays a plain `if`; do not contort a body
  to fit `const { }`.
- Dead-arm deletion is compiler behaviour, not a language guarantee: a 0 %-gate claim still needs
  its asm check.
- An associated const with a permissive default (`const SUPPORTED: bool = true`) inherits silently
  into every implementor; the `boyko_rhi` seam tests pin that inherited defaults REFUSE.

---

**Clause 4 (merged from ERG-33, third sweep) — every illegal `cfg` / feature combination is a
`compile_error!` per pair; every conditional site carries the axis banner; a build-variant witness
reports the configuration; a test-only substitution is a `cfg`-selected module of `pub use`
re-exports.** Binding: MUST for feature switches and tournament arms; MUST for loom / miri shims.
Cost: compile-time only; a `pub use` re-export is a name binding. Guarantee level: language. The
body below is ERG-33's, carried whole.

*Rule.* In `lib.rs`, one `#[cfg(all(feature = "x", feature = "y"))] compile_error!("<axis>: `x`
and `y` are mutually exclusive (<why>)")` per illegal pair. At every `#[cfg]` site of one axis the
same banner comment (`// === KE16 A switch: ke16-a1 / ke16-a1-fifo ===`), so the arm is greppable
and deletable as a unit. One `pub fn variant() -> String` printing the configuration — it allocates,
and that is permitted because it is called once per process at bench or gate entry. A model-checker
substitution is `#[cfg(loom)] pub use loom::sync::atomic::*` / `#[cfg(not(loom))] pub use
core::sync::atomic::*` — never a trait object, never a runtime flag. Custom `cfg`s are registered
in the workspace `unexpected_cfgs` table.

*Before* — two features that silently combine and produce a number for a configuration nobody
meant to measure; a loom shim behind a `dyn` atomic trait.

*After* — `crates/boyko_threadpool/src/lib.rs`: eleven `compile_error!` arms ("a mis-specified
`--features` line fails to BUILD instead of producing a number …") plus `ke16_variant()`;
`tls.rs`: the banner at each of its three conditional sites; `sync.rs`: the re-exports, with the
list of what is deliberately NOT shimmed and why per item; root `Cargo.toml`: `cfg(loom)` and
`cfg(force_alloc_panic)` registered.

*What it buys* — this repository's recorded failure mode is a gate that could not fail; a feature
combination that compiles and measures the wrong thing is that failure with a number attached.

*Verified* — `compile_error!` and `cfg` resolve before codegen; no runtime row exists or is needed.

*Exceptions* — the three that are ERG-33's own:
- `cfg(miri)` is built in and must NOT be registered in `unexpected_cfgs`.
- A pair Cargo already implies is legal and absent from the list; the list is edited when features
  change and deleted with the tournament.
- An unlisted omission from the shim is a hole; every deliberate non-shim carries its reason.
