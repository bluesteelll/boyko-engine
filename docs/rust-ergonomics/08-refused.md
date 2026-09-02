# §8 Refused — techniques this engine does not adopt, in BOTH directions

Part of [RUST-ERGONOMICS.md](../RUST-ERGONOMICS.md). Cost verdicts cite [EVIDENCE.md](EVIDENCE.md).

This section is as important as the rules, and it has two halves because the failure it guards
against is two-sided. **Part A** refuses techniques sold as "ergonomic and free" that COST here —
cycles, binary size, build time, a guarantee silently deleted, or a panic silently turned into a
wrong answer (REF-40 sits in Part A and REF-41 / REF-42 in Part B although numbered last — ids
are stable, not positional). It is what stops a developer
"improving" a hot loop into a regression. **Part B** refuses ABSTRACTIONS that cost nothing at
runtime and prevent no defect anyone here would hit — a name to learn and a hop to follow for
nothing. It is what stops the rules in §1 from being applied indiscriminately, which would make
them ignored. The deciding question for both halves is in the index.

Each entry: the technique · why it is tempting · the measured or structural reason · the verdict
and the rule that carries the permitted form.

---

## Part A — refused because it COSTS

**REF-00 — The hot-path ban list that no lint catches.** `clippy.toml` already denies `HashMap`
/ `HashSet` / `Mutex` / `RwLock` / `Rc` / `RefCell` / `parking_lot::*` / `hashbrown::*` under
`-D warnings` (registry row + rationale is the ONE exception route, `scripts/check_hotpath_exceptions.py`).
The half review must catch: `Box<dyn Trait>`, `Arc<Mutex<_>>`, `Vec::new()`, `format!`,
`String::from`, large `clone()` on the kernel and schedule rows. Exception route: a row of the
accepted-erasure list in the index with its amortisation unit. "It is cold" in a comment is the
sentence five falsified `type_intern` sites wrote. REFUSED (ERG-12, ERG-14, ERG-15, ERG-28).

**REF-01 — `Deref` on a domain newtype "to restore ergonomics".** Free at runtime — which is
why it is dangerous: it re-exposes every inner method and re-admits the raw value at every site;
on a solve view it reintroduces the O11-SP4 race in two lines (EV-17). REFUSED (ERG-17).

**REF-02 — Typestate through `Box<dyn State>` or a runtime `mode` field.** An allocation per
transition, an indirect call per method (EV-14: 5×; EV-60: 2.5–3× on 64-byte components), and
the wrong-state call becomes a runtime panic again — nothing was bought. REFUSED (ERG-08).

**REF-03 — `#[non_exhaustive]` on workspace-internal types.** Adding a variant produces ZERO
compile errors and N silent `_ =>` fallthroughs — a gate that cannot fail, made deliberately
(EV-26). REFUSED; the External row only (ERG-14).

**REF-04 — `#[must_use]` as enforcement.** 2 of 10 misuse shapes warn; `let _ = make_guard();`
— the actual RAII bug — is silent (EV-25). ADOPTED as a nudge (ERG-11), REFUSED as soundness;
load-bearing invariants are structural (ERG-06, ERG-08) or a `Drop` guard (ERG-43).

**REF-05 — GhostCell / branded lifetimes in the kernel.** The representation is free; the API
shape is not (an invariant `'id` infecting `System`, `SystemParam`, queries), and the model does
not fit: one exclusive token per brand proves "this thread owns all of it", the opposite of a
coloured solve over disjoint rows (documented, not measured). REFUSED; ERG-06 shape 2 is not this.

**REF-06 — `-> impl Trait` for a value the kernel stores; `Box<dyn Iterator>` as its fix.**
Unnameable, so the reachable "fix" is the banned box: allocation + vtable per `next()` (EV-13:
1.8×; EV-60: 2.8×). RPIT for consumed values; name the concrete type for stored ones (ERG-12).

**REF-07 — Const-generic `bool` flags; dimension-checked const-generic math; const generics
beyond two or three fixed capacities.** 2^N bodies per flag, one copy of every loop per shape;
the literal-argument runtime version folded to the same symbol (EV-19: 32 instantiations = 3×
code, +38 % build). `solve::<true, false>(dt)` is as unreadable as `solve(true, false)`.
REFUSED. Per-type predicates are associated consts under `if const` (ERG-31); two or three
fixed capacities with the padded-size gate (ERG-01) are permitted; a lane count is a plain `const`.

**REF-08 — Type aliases presented as type safety.** `pub type Generation = usize;` is transparent
to the checker and misleads the reader into assuming newtype behaviour. REFUSED (ERG-02).

**REF-09 — Public tuple fields on id newtypes.** `ComponentId(999_999)` is constructible
anywhere, so the type can never carry a proof and every `get_unchecked` keyed on it is unsound;
`define_id!` emits `pub` while `WorldId` in the same file keeps its field private. REFUSED for new
newtypes; the `define_id!` migration is an architecture pass (ERG-03).

**REF-10 — Third-party proc-macro crates for a `macro_rules!` shape** (`nutype`, `bitflags`,
`enum_dispatch`, `derive_more`). A dependency on every build, opaque generated code no `// SAFETY:`
can point at, and generated `Deref` / `From` / `Deserialize` impls that bypass the smart
constructor — a deserialised value is a forged proof. REFUSED (ERG-02, ERG-14).

**REF-11 — `#[track_caller]` on kernel functions.** An implicit `&Location` argument at EVERY
call site, and through a fn-pointer coercion it silently reports the DEFINITION site (EV-24) —
which would break the `CloneFn` / `HookFn` tables. REFUSED below the boot layer.

**REF-12 — Bare `PhantomData<T>` as a tag.** The one row that means "owns a `T`": auto-traits
are inherited, so a `Res` state becomes `!Send` the moment `R` is (EV-06). REFUSED (ERG-05).

**REF-13 — `..Default::default()` where `Default` allocates.** The overridden field's default is
built and then freed: 2 alloc + 1 dealloc per construction vs 1 + 0 (EV-45). REFUSED (ERG-29).

**REF-14 — `iter().chain(…)` inside a per-element loop.** A two-state iterator whose state test
survives into the loop: SMALLER and slower — 1.7× on an L1-resident loop (EV-21), 3–8 % at
column scale (EV-60). The magnitude is scale-dependent; the direction is not. REFUSED per element;
two loops (ERG-35). Fine at setup.

**REF-15 — `Index` as the hot-path accessor on a proven-in-range path.** `Index` cannot be
fallible or `unsafe`, so its contract is a compare and a cold panic branch per element (EV-23).
BUT the time delta of removing it did not reproduce on this engine's shapes (EV-60), so this is a
preference, not a licence: prefer the proven form — a proof-carrying id and a plain `[]`
(ERG-03; the mask that would delete the check is REF-40) — and never write `get_unchecked` on
this row's authority. KEEP `Index` for cold, diagnostic and test sites. The preference is for the
SEQUENTIAL shapes EV-60 measured; the gathered-index SIMD kernel (`colored.rs`,
`solve_color_avx2`, sixteen body rows per cohort off gathered indices) is UNMEASURED and stays
on `row_ptr` (ERG-03 exceptions, ERG-07).

**REF-16 — `#[inline(always)]` without a measurement.** A directive the compiler honours even
when honouring it bloats L1i (principle 3); rustc 1.97.1 already inlines tiny and ~20-line
cross-crate bodies with no attribute (EV-27). REFUSED without the line at the site (ERG-28).

**REF-17 — `FromIterator` / `collect()` into kernel storage.** "Construct a fresh collection from
nothing" is a reservation per collect or a `Vec` detour over a VM-reserved, address-stable
column — principle 0's parallel-data-system shape. Zero such impls today. REFUSED; `Extend` on
reserved capacity (ERG-36).

**REF-18 — Reflexive `#[derive(Debug)]` on large internal descriptors.** 3× the code, 7× the
string data, +43 % build for a 16-field struct plus a 30-variant enum (EV-34). Keep C-DEBUG on
public types; hand-write the three fields that matter on a 200-field GPU descriptor.

**REF-19 — Hand-rolled raw-pointer loops where a slice API exists.** The previous draft's
performance argument ("LLVM cannot prove two derefs off one raw base disjoint") is no longer
true on rustc 1.97.1 — the raw form vectorises and times identically (EV-60, EV-70). The refusal
stands on review budget: the raw loop carries a SAFETY obligation the slice form does not, for a
gain that is zero. REFUSED except ERG-24's two cases (no slice expresses the access; an aliasing
projection with its id).

**REF-20 — `impl Into<String>` / `String` in kernel signatures or errors.** An allocation with
a convenient face (EV-39 records honestly that the probe could not show it — LLVM deleted a dead
`String`), a body per argument type (EV-18), a 24-byte `Result` through every `?` (EV-54).
REFUSED below the boot layer (ERG-15, ERG-28).

**REF-21 — "Hoist config fields into locals before the loop" as a performance rule.** Through
`&Cfg` LLVM hoists the loads itself — 0 diff lines (EV-52). NOT A RULE: hoist for readability if
it reads better; never cite it as a performance change. (A refuted discipline is one of the
cheapest entries a guide can carry.)

**REF-22 — A lending-iterator trait as the query surface.** Not an `Iterator`: no `for`, no
`map` / `zip` / `sum`, `while let` everywhere; the published sketch failed under `-D warnings`
(EV-35). The tree already has the GAT it needs (`QueryData::Item<'w>`). REFUSED.

**REF-23 — A "static `!Send` assertion" that asserts nothing.** No stable negative bounds; the
placeholder compiles for any `T`. Only a trybuild case proves `!Send` (ERG-10, ERG-05).

**REF-24 — `thiserror` / `anyhow` / `Box<dyn Error>` in a kernel crate.** A heap allocation
and a vtable per `Err`, a proc-macro on every build for what ERG-28 writes in twelve lines, and
`#[from]` conversions that make an allocating `Err` reachable through `?` from any callee
(EV-54). REFUSED below the boot layer; a tools binary MAY use `anyhow` at `main`.

**REF-25 — A `Vec` / `HashMap` / `Box<[T]>` field inside a component or `Resource` as bulk
per-entity state.** A parallel data system (principle 0) — the O11-SP4 root cause was a
`std::Vec` physics mirror. REFUSED; dense components and the view split (ERG-07). Transient
function-local scratch that never crosses a parallel phase says so at the field.

**REF-26 — Blanket `impl<T: Bound> Trait for T` on `From` or an extension trait.** Coherence
conflicts, "type annotations needed" at call sites, and a silently chosen allocating conversion.
REFUSED in kernel crates; concrete impls only.

**REF-27 — `transmute` for layout punning without a gate.** Checks SIZE and nothing else. The
four permitted forms — lifetime erasure, a bit-pattern check beside a gate, a `repr(C)` POD
viewed as bytes beside its size AND `offset_of!` gates (`ddgi_update.rs`), the Vulkan PFN loader
cast and `MaybeUninit` assembly under a no-drop-glue check — each carry their SAFETY. (The
previous draft's census "none puns live kernel data" was wrong.) REFUSED otherwise (ERG-01).

**REF-28 — `OnceLock` / `LazyLock` read per element.** An acquire load and a compare that are
not hoisted (the miss branch is an exit), so the loop stays scalar — 8 instructions per element,
zero vector ops; hoisted, the plain vectorised loop (EV-56; EV-60: 1.6×). REFUSED per element;
`get()` once at the system entry. The one hoisting rule WITH a measurement — contrast REF-21.

**REF-40 — A range-narrowing mask (`& (N - 1)`, `% N`) on a proof-carrying id to delete the
bounds check.** Tempting because EV-67 shows the safe `[]` under the mask at 4 instructions
against 13 without it — no branch, no panic path — and because it looks like ERG-03 for free.
Refused on three counts. (1) On a valid id the mask does nothing; on a forged, stale or
wrongly-widened id it converts a bounds PANIC into a silent read of the wrong table row — which
is ERG-26 case 2's definition of silent corruption, in the component registry every storage path
keys on — so a developer cannot satisfy ERG-03-with-mask and ERG-26 at the same site. (2) The
4-vs-13 count is the reading this ledger's own rule forbids: instruction count is not a cost
proxy. (3) EV-60 measured no time delta from a bounds check on this engine's shapes. Plain `[]`
under the proof; `get_unchecked` only under a `// SAFETY:` naming the mint plus a measurement at
the site (ERG-03). The previous revision of this guide carried the mask as a rule; it is
withdrawn, and the withdrawal is recorded here so nobody re-derives it from EV-67's count.

---

## Part B — refused because it is CEREMONY

The evidence column for these is one sentence: "what defect would its absence let through?"
When the answer is "none", the type is a name to learn and a hop to follow.

**REF-29 — A newtype for a value that never crosses a boundary.** A `LaneCount(u32)` used
inside one function body has no seam at which something else could be passed. ERG-02 applies at
signatures and fields; inside one body the raw integer is correct. (The finder refused
`ThreadPoolBuilder::pin_workers(on: bool)` and the four named `EntityClonerBuilder` setters on
this test.)

**REF-30 — A trait with one implementor; a generic parameter with one instantiating type that
will ever exist.** No dispatch is decided, no second type is protected, and the reader follows
the hop to find one impl. Write the concrete type; introduce the trait when the second
implementor arrives. (EV-18 prices the generic form when the fan-out is real; with n = 1 it is
pure reading cost.)

**REF-31 — A builder for a POD config.** `Cfg { substeps: 2, ..Cfg::DEFAULT }` (ERG-29) is the
builder — a constant, functional update, no `build()` that can be forgotten and no
`#[must_use]` obligation. A builder earns its place when construction has ORDER or a required
field whose absence must not compile (ERG-08).

**REF-32 — A marker type, typestate or guard for a property the module boundary already
guarantees.** A `Drop` guard around a pairing with exactly one exit that cannot panic (ERG-43); a
typestate over an optional field ("combinatorics with no safety") or over five states (N×M
instantiations — ERG-08's principle-3 exception); a marker for a thread-affinity that a `!Send`
field already pins. Each states nothing the reader did not know.

**REF-33 — An enum over a `bool` whose method name already carries the noun.**
`with_hot_reload(true)`, `pin_workers(on)`, `strict(strict)`: the call site reads itself today.
ERG-39's mechanical decider — is the parameter's noun in the method name? — is the whole test;
sweeping the rule across builder setters is the ceremony it refuses. (`report_sv0_request_clamped(shadow: bool, ao: bool)`
is genuinely swappable and `#[cold]` diagnostics-only — a real defect with negligible consequence,
refused on that basis.)

**REF-34 — A data-carrying enum with a FULL-WIDTH payload as the STORED form of a same-width
sentinel.** `enum { Worker(u32), Dispatcher, Unattached }` is 8 bytes, not 4 — a plain `u32` has
no niche (EV-40, EV-64) — and so is `Option<WorkerId(u32)>`; a `NonZeroU32` payload buys exactly
ONE spare variant, so a three-state enum over it is 8 bytes too (EV-71). The orchestrator's
framing of the worked example ("the same four bytes with the same niche") is false FOR A `u32`
PAYLOAD — and that is the whole scope of this refusal. When the domain fits a narrower integer
(the worker id is `< MAX_WORKERS == 64`), the `#[repr(u8)]` enum with a right-sized payload is
2 bytes, smaller than the sentinel, and IS the stored form (ERG-04 shape 2a, EV-71). Only a
payload that needs its full bits takes ERG-04's transparent-plus-view split, with the view a
temporary. Ask "does the payload need all its bits?" before reaching for either.

**REF-35 — A container abstraction where newtyping the elements buys the invariant.** A
`Csr<K, V>` over `ConstraintGraph`'s offset / value pairs adds a name and a hop and must be
threaded through the coloring pass; `CsrOffset` / `ManifoldIdx` newtypes on the ELEMENTS are free
and make five of the six confusions a compile error (EV-59). Likewise a units-of-measure layer over
`boyko_math`'s components: real reading cost, no defect class here, and the crate is
bit-determinism-pinned.

**REF-36 — Slice patterns, `first_chunk`, `as_chunks` or any adaptor as a PERFORMANCE claim.**
The ledger's history forbids it: EV-20's four adaptor comparisons came out three ways, EV-21's
smaller code was slower. A refutable slice pattern on a runtime-length slice is a let-else with an
`else` arm the reader must evaluate; `as_chunks` drops the remainder silently. Use them where they
READ better (ERG-38); never cite them as a cost change without a row.

**REF-37 — `SeqCst` as the default ordering; `compare_exchange` strong-vs-weak as an x86 cost
argument.** The first is a locked RMW per store (EV-63); the second is one instruction either way
on x86 and a portability question only. ERG-45 owns both.

**REF-38 — `#[must_use]` sprayed across pure methods by `clippy::must_use_candidate`.** The lint
fires on essentially every pure method; decorating all 49 `boyko_math` `pub fn`s is a maintenance
tail for a nudge that catches 2 of 10 misuse shapes (EV-25). ERG-11 keeps the attribute on TYPES
and on statuses whose drop is a bug.

**REF-39 — A `macro_rules!` forced by counting to three.** Three types that merely look alike are
not a family; the macro hop degrades goto-definition, rustdoc and grep-for-the-impl, and generated
`unsafe` cannot carry a per-site `// SAFETY:`. ERG-02's macro clause is a SHOULD triggered by
"these siblings must not diverge" (`define_id!`, `code_newtype!`, the six RHI flag families).

**REF-41 — A newtype over a lone integer that is only counted or compared.**
`wake_after_push(inner, pre_len: usize)` compares it with `> 1`; `with_capacity(ids)`,
`grow_to(id)`, `num_threads(n)`, `manifold_fill(n)` hand it to a `resize` or a clamp;
`injector_pre_len` / `deque_pre_len` return one. Each crosses a boundary, so REF-29 does not
exempt it — but the method name carries the noun, nothing of the same width could be confused
with it there, and it is neither stored nor used as an index. A `QueueLen(usize)` is a name to
learn for no defect prevented, and a MUST that fires on it is a MUST a developer learns to
skim. ERG-02's three-part test (stored; a same-typed positional pair; indexes or is compared
across domains) is the decider, parallel to ERG-39's noun-in-the-name test for a `bool`; a lone
count with its noun in the name fails all three and stays bare.

**REF-42 — A trybuild fixture for a refusal the language cannot lose.** An `E0308` between two
distinct nominal newtypes (the transposed `claim_one_idle` call) cannot stop being an error
without a `Deref` (ERG-17) or a `From`; a `.stderr` for it pins nothing that can change and
costs a re-bless on every rustc move — 23 fixtures went red at once on one bump
(`docs/OPEN-QUESTIONS.md`, 2026-08-11), and the corpus already needs a witness against an empty
glob. ERG-10's MUST is for properties an `impl`, a derive or a method can silently undo: a
`!Send` pin, an operator added to `IdleWord`, a slice method on a solve view, a wrong-phase
call, a downstream impl of a sealed trait.
