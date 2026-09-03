# §8 Refused — techniques this engine does not adopt, in BOTH directions

Part of [RUST-ERGONOMICS.md](../RUST-ERGONOMICS.md). Cost verdicts cite [EVIDENCE.md](EVIDENCE.md).

This section is as important as the rules, and it has two halves because the failure it guards
against is two-sided. **Part A** refuses techniques sold as "ergonomic and free" that COST here —
cycles, binary size, build time, a guarantee silently deleted, or a panic silently turned into a
wrong answer (REF-40 sits in Part A and REF-41 / REF-42 in Part B although numbered last — ids
are stable, not positional; REF-43 … REF-45, added by the second technique sweep, REF-46 … REF-49, added by
the third, and REF-50 … REF-52, added by the fourth, are appended at the end of Part B — of those, REF-43, REF-44 and REF-47 are Part-A
refusals by nature and the rest, REF-50 … REF-52 included, are Part-B; again, ids are stable, not positional). It is what stops a developer
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
*Added by the fourth sweep, as a NEGATIVE result worth having on record:* there is no type-level
spelling of "this function may not allocate". Rust has no negative bounds and no effect system, a
`!Alloc` bound cannot be written, and REF-23 already records that a static `!Send` assertion
asserts nothing. The half this ban leaves to a reviewer therefore has exactly three mechanical
answers, none of them a type: a capability TOKEN that must be presented to do the dangerous thing
(ERG-06, `DispatcherToken`); a crate that does not LINK `alloc`, in which four of the five
un-lintable bans are not NAMEABLE (`#![no_std]`, index OPEN 13, priced at EV-105 and EV-113); and a
LINKER refusal for the panic half (index OPEN 16). Two of the three are already open items with
measurements attached, and neither is an ergonomics rule — which is why this refusal stays
review-enforced and says so.

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
fixed capacities with the padded-size gate (ERG-01) are permitted; a lane count is a plain `const`,
and a capacity taken FROM a file (`SpirvBlob<{ include_bytes!(..).len() }>`, ERG-01 clause 7) is the
second permitted shape, because its fan-out is the number of blobs rather than a product.
*Two notes added by the fourth sweep.* (1) **The strongest external confirmation this refusal will
get comes from the community that most wanted the opposite:** `heapless` — whose whole thesis is
capacity-in-the-type — shipped View types in 0.9 (`VecView<T>` is to `Vec<T, N>` what `[T]` is to
`[T; N]`, reached by unsizing coercion) for the stated purpose of improving "both compile time and
the size of the resulting binary". That is an escape hatch from exactly the cost EV-19 measured.
It is NOT adopted here: the only const-generic container in this tree is `boyko_log`'s
`DspBuf<N>` at two plausible instantiations, which REF-07 already permits, so a `?Sized` view
sibling would be machinery for a fan-out of two. It is recorded so that a THIRD capacity is a
signal to build the view rather than to argue with this refusal. (2) **Type-level integers
(`typenum`, Peano `Succ<Zero>`) are this same refusal by a worse mechanism** and are covered by it:
min-const-generics has expressed the motivating case since 1.51, the encoding gives the part of the
language responsible for arithmetic a second incompatible counterpart that must be written twice,
and the errors are trait-resolution failures with no arithmetic in them. No site exists —
`boyko_math` is fixed-size, the physics kernels are fixed 8-wide, and every kernel capacity is a
plain `const`.

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

**REF-43 — `core::hint::cold_path()` in a branch whose slow arm is already a `#[cold]` fn.**
Tempting because ERG-28 expresses coldness only at FUNCTION granularity, and a rare arm that
cannot be outlined without a call appears to get nothing — so the in-body hint looks like the
missing half. It is not, at the shape that motivated it. MEASURED (EV-89) on
`crates/boyko_ecs/src/ecs/memory/component_pool.rs`'s `add_typed` growth arm, whose `grow_rows`
is already `#[cold] #[inline(never)]`: with and without `cold_path()` the bodies are **0 code
diff lines** at codegen-units 1 and 16 — the only cu-16 difference is the `@feat.00` / `.text`
assembler directives — and the `callq grow_rows` sits at the same block position in both. The
`#[cold]` on the callee already told LLVM everything the hint would. std's own doc adds the
falsifier: it "can slow a path that is taken more often than expected". REFUSED as a rule; it is
a per-site directive under ERG-28's measurement line like `#[inline(always)]`, and a site that
adopts it must show a block-order or timing change the `#[cold]` callee did not already produce.
(`cold_path` IS stable on `rustc 1.97.1` — probed, along with `assert_unchecked` and
`select_unpredictable`; the "nightly" label one sweep gave it is wrong.)

**REF-44 — A HOISTED `core::hint::assert_unchecked` on a gathered-index MAXIMUM.** The proposed
shape was one UB promise at the top of the cohort loop —
`assert_unchecked(max_gathered < bodies.len())` — with every per-lane `[]` left safe and its
branch "now provably dead". It is not dead, and the promise costs. MEASURED (EV-92) on the
`solve_color_avx2` gather shape: the safe `[]` form is 261 instructions with **13 panic sites**;
the same form under the hoisted promise is **371 instructions with the same 13 panic sites** —
LLVM cannot connect a recorded maximum to `a[lane]`, so nothing is discharged and the body grows
by 42 %. Only the promise repeated PER LANE elides the checks (109 instructions, 0 panic sites),
and at that point it is `get_unchecked` (107) with a different spelling and the identical UB on
violation. REFUSED in the hoisted form. The per-lane form is not separately refused and not
separately permitted: it is exactly what ERG-03 already licenses — a measurement at the site and
a SAFETY naming the mint — with no additional guarantee, so a site that wants it writes
`get_unchecked` and gets the same asm, or writes the safe `[]`, which EV-92 measured at
**0.76–0.79× of the current `row_ptr` form** in that shape (index OPEN 9).

**REF-45 — `ParamSet` as a rule of THIS guide.** A `ParamSet<(P0, P1)>` whose `p0(&mut self)` /
`p1(&mut self)` accessors make two otherwise-conflicting system parameters non-simultaneous is
free and it works: `via_set` is 110 instructions against 121 for direct field access, and holding
`p0`'s item across a `p1()` call is `E0499` (EV-94). It is refused here anyway, on the guide's own
deciding question. It is not a SPELLING of something the tree already writes — there is no
`ParamSet` in `crates/boyko_ecs/src/ecs/core/system/params/`, and the conflict detector in
`filtered_access_set.rs` can today only REJECT, so adopting it is building a scheduler feature,
not restating an invariant. No author is on record hitting the rejection. A guide that grows by
absorbing unbuilt features stops being a guide; this belongs in the schedule campaign's backlog
with EV-94 attached, and returns as an ERG-06 INSTANCE if it is ever built.

**REF-46 — The leak-on-panic guard for an IN-PLACE column rewrite (rustc's `flat_map_in_place`
shape).** ERG-43 teaches the RESTORING guard; its counterpart — a guard that deliberately does NOT
restore, because restoring mid-move is unsound and setting `len = 0` and leaking is the only
correct unwind behaviour — is genuinely counterintuitive and is not in the guide. It is refused
here **for want of a site, which was the outcome the sweep itself said was possible.** The three
candidate files were read on this checkout with no build: `entity_master.rs`'s two `collect()`
calls are both BELOW its `#[cfg(test)]` boundary and are test scaffolding;
`migration_helpers.rs`'s one production `collect()` builds a short `Vec<ComponentId>` of retained
ids at migration-planning time — it allocates a NEW list, it does not rewrite a column in place,
and there is no half-moved state for a guard to be correct about; `component_pool.rs`'s
`swap_remove` is the single-element case, which has no unwind window at all (`T: Copy`, two
writes kept together under ERG-20). A rule whose Before does not exist is a rule nobody can apply.
Recorded rather than dropped, because the mechanism is right and the day a column IS rewritten in
place — the Gaia bake's load path is the plausible first one — this entry is where to look, and
the shape is: set the length to 0 first, rebuild, `mem::forget` the guard once the buffer is valid
again.

**REF-47 — "Push ifs up and fors down" as a rule of this guide, on the strength of its named
site.** The technique (hoist a branch above a function boundary; make the BATCH rather than the
element the primitive) is real advice from a credible source, and the sweep pointed it at
`crates/boyko_physics/src/systems.rs`'s AVX2 pass 2, where a per-corner `if d >= 0.0 { continue }`
sits around an 8-wide SDF call. **MEASURED and refuted at that site** (EV-106): the compacted
two-pass form is **171 instructions with 104 vector ops and one panic site** against the branchy
form's **127 / 113 / 0** — bigger and LESS vectorised — and timing under load, best-of-7 × 3
process runs, at the three penetrating-corner ratios that could decide it gives **0.96×–1.03×**,
with no win at any ratio in any run. The reason is structural and worth stating so nobody
re-proposes it: each surviving corner needs its OWN six-offset gradient evaluation, so compacting
the indices does not create a wider vector op — it only adds a gather and a second loop. REFUSED
for want of a site rather than on the merits of the idea; a site where the per-element body is
genuinely lane-uniform would be a different measurement. The one half of the advice this guide
already carries is ERG-35 clause 5 (`&` over `&&` where the predicate feeds an accumulator).

**REF-48 — `Fn` instead of `FnMut` as a capture-restriction tool.** rustc's `unord.rs` takes
`impl Fn(&K, &V)` rather than `impl FnMut` so the closure cannot ACCUMULATE across calls — "to
reduce the risk of accidentally leaking the internal order via the closure environment" — and the
tightening is free (a bound change emits nothing; EV-66 already established the scoped-closure
form as an ICF alias). It is refused as ceremony because **the census says there is nothing to
tighten**. Counted on this checkout: **26 `FnMut` bounds across 22 files** under
`crates/boyko_ecs/src/ecs/core` and **2** in `crates/boyko_threadpool/src`, and essentially every
one is a `for_each`-shaped callback whose caller legitimately mutates — `Query::for_each` and
`for_each_chunk` (a user body accumulating), `for_each_component_bytes` /
`for_each_data_component_bytes` (writing into the destination),
`for_each_required_id_excluding` and `for_each_set_bit` (accumulating), `ExclusiveFunctionSystem`'s
`FnMut(&mut EcsMaster)`. The motivating problem does not exist here either: the order-leak
`unord.rs` guards against is hash-iteration order, and `clippy.toml` bans the map types on these
rows. REF-32's neighbourhood — a restriction for what the surface already guarantees.

**REF-49 — Nested-tuple folding in a derive as an alternative to a compile-time arity refusal.**
ERG-01 cites `crates/boyko_ecs/src/ecs/core/system/params/tuple_impl.rs`'s
`const { panic!("MAX_SYSTEM_PARAM_ARITY = 12") }` as its exemplar, and Bevy's
`derive_system_param` shows the other answer: fold fields into nested tuples above the limit, so
the user never meets the ceiling. The DISTINCTION is real and cheap to state — a compile-time
refusal is right when the user can restructure, wrong when the macro could have restructured for
them. It is refused because **`boyko_macros` has no derive that can hit an arity ceiling**: its
eight derives (`Component`, `Relationship`, `RelationshipTarget`, `Resource`, `Bundle`,
`SystemSet`, `Actionlike`, `Bindable`) all emit per-FIELD code, not a tuple type, and the word
"tuple" appears in that crate only in `compile_error!` text about struct shape. The one place a
ceiling is met is a bare 13-argument `fn` system, which cannot be folded because there is no
derive to do the folding — so `tuple_impl.rs`'s `const { panic!() }` stays correct and this would
be a clause about macros that do not exist. Recorded so the distinction is written down for the
day a composite derive (`SystemParam`, `QueryData`) is actually built; it belongs in that
campaign's plan, next to REF-45.

**REF-50 — Phantom SOURCE / DESTINATION spaces on a transform type (`euclid`'s
`Transform3D<T, Src, Dst>`).** The temptation is real and the defect class is real: a 4×4 matrix
carries no evidence of which two spaces it maps between, `boyko_render/src/view.rs`'s
`pub fn view_proj_columns(m: Mat4)` takes a bare `Mat4` that MUST be a view-projection and cannot
say so, and this repository's recorded render failures — the net-Y-inversion rule, the shadow
mirror rule, a pose drawn a frame late — are the family a wrong-space multiplication belongs to.
It is also **free at run time**, which is why it is refused HERE and not in Part A: MEASURED
(EV-114), `Tf<Src, Dst>(Mat4, PhantomData<fn(Src) -> Dst>)` with a `then` composition against the
bare `proj.mul(view)` gives **`view_proj_spaced = view_proj_bare`, an ICF alias at codegen-units 16
AND 1**, 17 instructions, and `Tf<World, View>` is 64 bytes / align 16 exactly like `Mat4`.
What pays is the API surface: two parameters on every constructor, every `Mat4` method and every
GPU upload seam, across **276 space-named matrix occurrences in 13 files of `boyko_render/src`**
(view.rs 91, shadow_atlas.rs 60, hzb.rs 32, csm_config.rs 30, motion_cam.rs 27, frustum.rs 19), in
a crate whose math is bit-determinism-pinned against a shader oracle so every touch is
identity-gated. REF-35's reasoning applies unchanged — a container abstraction where newtyping the
value buys the invariant — and the cheaper answer at the site that motivated this is already a rule
of the guide: `view_proj_columns(vp: ViewProj)` over a `#[repr(transparent)] struct ViewProj(Mat4)`
minted where the product is formed (ERG-02). That states the one fact the seam needs, in one file,
for one type. REFUSED as a SYSTEM; the per-seam newtype is ERG-02 and is encouraged.

**REF-51 — A bbqueue-style write GRANT whose `Drop` publishes nothing.** The mechanism is right
and is genuinely not in this guide: `grant_exact(n) -> GrantW` hands out the region, `commit(used)`
CONSUMES the grant and is the only way to publish, and dropping it publishes NOTHING — the mirror
image of ERG-48 shape 4's ticket, whose drop is a tripwire because there the correct unwind state
is "give it back", and the mirror image of REF-46's leak-on-panic guard, which is refused for the
same reason as this one. `boyko_log`'s own prose names the hazard — "the open-record window"
appears in `record.rs` twice and in `sink/request.rs` once, and `DspBuf` exists "to keep user code
out of" it — so the site looked certain. **It is not there.** `lane.rs::emit_to` was read on this
checkout: `admit` is a pure budget computation that mutates nothing, the header and payload are
written into bytes the consumer cannot yet see, and the cursor advances in ONE `lane.write.store(…,
Release)` that is the function's last statement. There is no reserve→commit window: every early
return is before the publish, the arguments are `Copy` so encoding runs no destructor, and "the
record never existed" is already the unwind state, structurally, with no type needed. A `Grant`
here would be a type that states what the control flow already guarantees — REF-32. RECORDED
rather than dropped, because the mechanism is correct and the day a ring in this engine DOES
advance a cursor before it fills the region (a DMA- or GPU-filled staging span is the plausible
first one), this entry is where to look, and the shape is: the reservation is a returned value,
`commit(used)` consumes it, and `Drop` does nothing at all.

**REF-52 — A const-constructible type id (`typeid::ConstTypeId`, `#![feature(const_type_id)]`) to
fold `TypeIntern`'s runtime hash into a call-site constant.** Tempting because the cost is visible:
`boyko_utils::type_intern`'s `KeyHasher` mixes a 128-bit `TypeId` with fxhash on EVERY registry
lookup, four registries key on it, and `TypeId::of::<T>()` is not const-constructible on stable, so
a per-`T` `const KEY: u64` cannot be written. Refused on the price, which the stand-in's own
documentation states: unlike `core::any::TypeId`, matching `ConstTypeId`s do NOT guarantee
identical types outside a stated special case — and the upstream stabilisation of the std version
is blocked on precisely that ("a new scheme for building type ids that is collision resistant is
needed before stabilizing", rust#77125 / #144133). A key collision in `component_registry` is not a
wrong answer at one site; it is two components sharing a storage row in the table every storage
path resolves through, which is the KE13 defect class at its worst. EnTT documents the same failure
mode for its constexpr `type_hash`. It is additionally a third-party proc-macro-adjacent dependency
on the kernel's boot path (REF-10). REFUSED on stable; the nightly `const_type_id` form is a
`RUST-FRONTIER.md` BLOCKED-ON-STABLE row, not an ergonomics rule. If the hash ever measures, the
answer that does not weaken uniqueness is to hoist the lookup out of the loop (REF-28's shape), not
to make the id constant.
