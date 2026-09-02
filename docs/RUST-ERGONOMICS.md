# RUST-ERGONOMICS — the binding guide to free compile-time strength in boyko-engine

**The goal, in the owner's terms.** Rust has a pile of features that make code more elegant and
readable and the type system stronger WHILE KEEPING PERFORMANCE. Its compile-time strength —
newtypes, niches, enums, lifetimes, `Drop`, `const`, exhaustiveness — is largely FREE at runtime:
the machine code is the same. Leaving it unused — writing a from-scratch engine with C discipline
and Rust syntax, so an invariant lives in a comment, a variant in a magic constant, a proof in the
author's head — is as much a defect as piling on abstraction. This guide is two-sided by
construction: it refuses gratuitous abstraction (a name to learn and a hop to follow that prevents
nothing) AND it names the C-in-Rust patterns, which are the failure this engine is more exposed to
because they are invisible — the code works, it is fast, and nothing complains.

**Two bars every rule passed.** (1) It makes an invariant VISIBLE or a defect class IMPOSSIBLE,
and the rule says which. (2) Its cost was COMPILED AND COMPARED in both spellings on this
checkout's toolchain (`rustc 1.97.1`, `stable-x86_64-pc-windows-gnu`, edition 2024) — every
number is a row of [rust-ergonomics/EVIDENCE.md](rust-ergonomics/EVIDENCE.md), and a rule never
carries a number that is not in that ledger. Where a previous number did not reproduce, the rule
says so and the old row is kept as superseded.

**The shipped ISA is `x86-64-v3` (AVX2 + FMA), and this document said otherwise until
2026-09-02.** `.cargo/config.toml` sets `-C target-cpu=x86-64-v3` on all three x86 targets (owner
ruling of that date, "AVX2 by default"). Rows EV-01 … EV-76 were taken at the bare x86-64
baseline: their IDENTITY results are ISA-independent and stand, their ABSOLUTE ns/element figures
were taken at SSE2 and do not transfer to the shipped build; they are read as ratios. Rows
EV-77 … EV-107 were taken at `v3`. ⚠️ **Every timing in EV-77 … EV-107 was taken with the box
under load** (a game plus three concurrent build workflows); each is best-of-7 across three process
runs with all three printed. Directions reproduced; magnitudes did not. Every IDENTITY in
EV-96 … EV-107 was taken at the SHIPPED `codegen-units = 16` and re-checked at 1, and the row says
which — an ICF alias at one unit proves nothing at sixteen.

**Precedence.** [CLAUDE.md](../CLAUDE.md)'s principles win any conflict, and the rule says so
where they touch (the yields table below). Code is cited by file path and item name, never by
line number.

**Size.** **Thirty rules** — still thirty after the third technique sweep as well, which is the
point. The third sweep (2026-09-03) added ONE rule, **ERG-48** (an obligation no representation
can hold is a bound or a returned value), and **paid for it by merging ERG-33 into ERG-31 as
clause 4** — the section's own title already pairs const evaluation and `cfg`, both rules say
"make the compiler SEE the decision so a configuration nobody meant cannot exist at runtime", and
ERG-29 was merged into the same rule on the same reasoning one revision earlier. ERG-33's body,
Before, After, what-it-buys and all three exceptions are carried whole. Six further findings were
adopted as CLAUSES on existing rules (ERG-01 clauses 6 and 7, ERG-04 clause 5, ERG-20 clause 6),
four were refused (**REF-46 … REF-49**, so **fifty refusals**, REF-00 … REF-49), two became
architecture items with measurements (index OPEN 10's parity arm, OPEN 13), two became
`RUST-FRONTIER.md` rows this pass was scoped out of writing (OPEN 14), and one was a **correction
to an adopted clause of the previous sweep**: ERG-22 clause 4's roundtrip half is WITHDRAWN,
because its sibling ledger already refused it with a Miri measurement (FR-13 / FV-12) while the
clause said the benefit "was not measured" — the repository's own recorded failure class, a
summary outliving its refutation, found inside the guide that names it.

The second sweep's own size paragraph, unchanged and still binding: thirty rules — still thirty
after that sweep too, which is the point:
the target this document sets itself is twenty to thirty, a developer reads what he can hold, and
a guide that grows quietly stops being read. The sweep added two rules (ERG-46 featured island,
ERG-47 derive-induced bounds) and **paid for both by merging two**: ERG-09 (sealing) into ERG-20
clause 5, because both answer "who may establish an unsafe obligation, and where is that written"
— the guide's own precedent is ERG-21; and ERG-29 (what can be `const` is `const`) into ERG-31
clause 1, because the two were one rule wearing two ids and ERG-29's `const fn` accessor exists
precisely so the value can reach ERG-31's call-site constant. Nothing was cut: both merged bodies
are carried whole, with their Befores, Afters, EV rows and exceptions. Twelve further findings
were adopted as CLAUSES on existing rules rather than as new ids — fourteen clauses in all — and
three were refused (REF-43 … REF-45). Of the previous draft's forty-two rules, fourteen were
merged or cut; every retirement is recorded with its reason at the end of this file so the record
of why survives. A second revision answered a confirming reader's seven objections, each settled
by compiling (EV-71 … EV-73): ERG-03's range-narrowing mask is withdrawn (REF-40); ERG-04 gains
the right-sized-payload form and names the second role encoding in `boyko_diag::lane`; ERG-02
names `claim_one_idle` as a Before instead of exempting it; ERG-06 names `SharedPtr` as a
Before instead of ERG-20 citing it as an After; ERG-01's `from_raw_parts` trigger fires only on
a layout claim; and no example in this guide spells an infallible view accessor `get` or hands
a lane out as a bare integer. A third revision answered the reader's second set of seven, again
by compiling (EV-74 … EV-76): `WorkerId` now states ONE bound (`< 64`, the fact both its mints
establish) and ERG-03 no longer claims it discharges `< worker_count`; ERG-02 gains an integer
decider parallel to ERG-39's `bool` decider, so a lone count with its noun in the method name
stays bare (REF-41), and a `[lo, hi)` pair is a `Range<T>`; ERG-03 gains the reachable-sink
shape behind the two largest measured threadpool defects; ERG-10's MUST is scoped to properties
an impl can silently undo (REF-42); ERG-28 gains the failure-versus-predicate decider; and
ERG-03's cost sentence is scoped to the sequential shapes EV-60 measured, naming the gathered
SIMD kernel as unmeasured (OPEN 9).

A fourth revision (2026-09-02) folded in a second technique sweep cut by SOURCE rather than by
language section — where advanced Rust is practised, rather than what a table of contents lists.
Of roughly seventy-five candidates it kept twenty, and of those twenty **every one was compiled
in a lab outside the repository at the shipped profile before it was written down** (EV-77 …
EV-95): two became rules, twelve became clauses (fourteen clauses in all), three became
refusals, one became an architecture item with a measurement attached (OPEN 10), one became a
method row in the ledger (method S), and one was a correction to this document's own ISA claim. Three classes it found that a section-shaped survey cannot:
**(a) SUBTRACTIVE language features** — `target_feature_11` (1.86), `core::hint::{cold_path,
assert_unchecked, select_unpredictable}` (1.81 / 1.88 / 1.95) and the Strict Provenance APIs
(1.84) exist to DELETE obligations the unsafe rules currently teach how to document; the tree is
half-migrated on the first, which is the worst state, because the surviving prose is now false.
**(b) DEFAULTS that undo a deliberately-chosen invariant** — `#[derive]`'s bound on a generic
parameter (ERG-47, fifteen sites, the correct form already in tree at one), `usize` as an id
width (ERG-02 clause 5, a 16-byte `Entity` with four bytes of padding), and `64` as a
false-sharing pad (ERG-01 clause 4, thirty-nine sites against `CachePadded`'s 128 in seven). The
guide caught C-in-Rust; it did not yet catch Rust-defaults-in-Rust. **(c) The instruments behind
its own numbers** — `-Zprint-type-sizes` is now a named method (S); `cargo llvm-lines`, which is
the instrument REF-07 and REF-18 actually needed, is NOT installed here and is therefore NOT
claimed.

A fifth revision (2026-09-03) folded in a THIRD sweep, cut by source again, and its marginal yield
was small and sharply shaped — which is itself the finding worth recording. Of roughly seventy-five
candidates diffed against this guide, about two thirds were already rules, clauses, refusals,
evidence rows or open items, site-for-site and number-for-number. What survived names ONE class the
guide had almost no vocabulary for, and every survivor is in it: **an invariant that no
REPRESENTATION can hold** — a property of a value STREAM ("these indices are pairwise distinct"),
of a BUFFER'S HISTORY ("these bytes are aligned for `T`", "this range was validated"), or of an
EXTERNAL AGENT'S TIMELINE ("the device is still writing here"). Every one of the other thirty rules
puts its fact into a value's representation: a niche (ERG-04), a width (ERG-02), a marker row
(ERG-05), a bound (ERG-03, ERG-31), a lifetime (ERG-06), a layout (ERG-01). That axis is finished,
which is why a source-cut survey now returns mostly confirmations. Rust's tools for the other axis
are the sealed type parameter, the empty `unsafe` marker trait and the consuming call — **ERG-48** —
and the cost of not having had them is measurable in prose: 118 "fence-waited" preconditions across
23 RHI files, ten of `colored.rs`'s fifty `// SAFETY:` blocks restating one coloring invariant, and
a named `CQ-PACK1` invariant written out four times in `command_queue.rs`. ⚠️ **A fourth
source-cut sweep is not recommended**: two of this one's four defensible items were each found by
two different lenses, which is the signal that the well is being read twice. The axis no external
source can supply is this document's own OPEN list and the diagnostics campaign's "gate that could
not fail" family — cut the next pass by DEFECT CLASS.

## The deciding question

For every candidate type, wrapper, trait, generic or guard — not "is it an abstraction" but:

> Does this use Rust's compile-time strength to make the code clearer or safer, at the SAME
> machine code — and would its absence let a real defect through, or leave a real invariant
> unstated?

Two questions, both of which must answer yes before a type is added:
1. **Does the value CROSS a boundary** — a signature, a struct field, a thread, a frame — where
   something else of the same representation could be passed instead? A value that never leaves
   one function body has no confusion to prevent (REF-29).
2. **Does the type make a specific wrong program STOP COMPILING**, and can you name that program in
   one sentence and write its trybuild case (ERG-10)? If the sentence cannot be written, the type
   states nothing the reader did not already know — ceremony, refused.

Most kernel values pass both; that is the correct outcome. A technique that is free AND makes the
invariant visible is exactly what the owner is asking for. A technique that is free but states
nothing is refused (§8 Part B).

## THE OTHER FAILURE — C-in-Rust, recognisable in your own diff

Each pattern below is in this tree today, is fast, works, and is wrong in a way nothing complains
about. The free spelling beside it was compiled and compared (the row is named in the rule).

| You wrote | Where it is in this tree | The free Rust spelling | Rule |
|---|---|---|---|
| A bare `u32` / `usize` that is stored, paired with a same-typed operand, or used to index — a component id, a row, a stride, a worker id | `ComponentPool::new(component_id: usize, reserve_rows: usize)`; `create_column(stride: u32, cap_rows: u32)`; `claim_one_idle(observed: u64, exclude: u64, start: u32) -> Option<u32>` (forty lines of doc on why the two masks must not be confused; the confusion compiles); `prev_labels: Option<(u32, u16)>`; callers writing `.get()` at the seam | a `#[repr(transparent)]` newtype per domain, by the three-part test (a lone count with its noun in the method name stays bare — REF-41); a same-typed pair as a PAIR (`IdleWord` / `ExcludeMask`, the first with no `BitAnd`); two same-typed positional integers in one signature is the tell | ERG-02 |
| A `[lo, hi)` pair as `(usize, usize)`, destructured by hand at every use | `solve_color(.., span: (usize, usize), ..)`, `solve_color_dispatch(.., span, g_lo: usize, g_hi: usize, ..)`, `let (start, end) = span; for i in start..end` (`colored.rs`) | `core::ops::Range<usize>` — same 16 bytes, same loop; the order is stated where it is built, `len()` saturates, `&xs[span]` hoists the bounds check (a reversed range still compiles — a SHOULD) | ERG-02 |
| A magic constant standing in for a variant, compared by hand at every site — or the same fact stored twice in two integer encodings kept in step by "site N of 3" comments | `WORKER_ID_DISPATCHER = u32::MAX - 1` / `WORKER_ID_UNATTACHED` (tls.rs, three spellings of one question) beside `LANE_DISPATCHER = 64` / `LANE_HOST` / `LANE_UNCLAIMED = u16::MAX` (`boyko_diag::lane`), synchronised by the D1 three-site discipline; `ABSENT = u32::MAX` with nine comparisons; `NO_ISLAND`; sixteen `X::MAX` sentinels across nine crates | absence → `Option<NonZeroU32>` (same 4 bytes); a mode whose payload fits a narrower integer → a `#[repr(u8)]` data-carrying enum (2 bytes, smaller than the sentinel) with `lane()` deriving the second encoding where it is derivable; a full-width payload → a stored `#[repr(transparent)]` newtype with a view enum | ERG-04 |
| `Option<usize>` for an optional index | `SparseMap::sparse: Vec<Option<usize>>` — 16 bytes per slot, 4× the memory, 2.55× the probe time | `Option<NonZeroU32>` — the niche IS the absent state | ERG-04 |
| A `bool` parameter at a call site that says nothing, or a `bool` return meaning two unrelated things | `match_word(p, w, !want)` (a stray `!` is a silent wrong answer); `set_enable_bit(.., true) -> bool`; `create_fence(true)` | a two-variant enum (`Polarity`, `EnableBit`, `FenceState`) — one instruction shorter | ERG-39 |
| A `bool` failure signal, accumulated with `&=` | `pool.swap_remove(row) -> bool`; the bundle keeps mutating past the first failure | `Result<(), FieldlessEnum>` — one byte, same instructions, `#[must_use]` by construction, `?` stops | ERG-28 |
| A closed set as `pub const u32` constants | `sdf_op::{UNION, SUBTRACT, INTERSECT}`; `boyko_log`'s `class: u8` under a doc titled "class is a TYPE, not a field" | a `#[repr(u32)]` enum at the API surface; the wire field stays the integer | ERG-14 |
| A null-sentinel raw pointer checked by hand, then arithmetic that trusts the check | `Column { ptr: *mut u8, .. }` + `is_null()`; `VkBuffer(pub u64)` + `NULL` | `Option<NonNull<u8>>` — same 16-byte layout, `zeroed()` is `None`, `let base = col.ptr?` PRODUCES the proof | ERG-22 |
| An invariant tracked in a comment or a debug-only flag | `LoadEntityMap { #[cfg(debug_assertions)] sealed: bool }` — in release, `get` before `finalize` is an unsorted binary search; `WorkerLane.wid: u32` with "always `< worker_count`" in a doc | a ZST typestate (24 bytes, was 32); a `WorkerId(u8)` minted only where `< 64` is established — the bound it states is the bound it proves, no more; `1u64 << id` cannot mark the wrong worker | ERG-08, ERG-03 |
| A `Copy` handle handing out a reference with a FREE lifetime, and "MUST NOT be used after this line" in a comment | `SharedPtr::as_ref<'a>(&self) -> &'a ScopeShared` (scope.rs) and the task wrapper's "`shared` MUST NOT be dereferenced after this line" — on an allocation that has already had a use-after-free (Phase 9.2 Candidate U) | a non-`Copy` handle whose last access `complete(self)` consumes it — the same 22 instructions; a later use is `E0382` | ERG-06 |
| A hand-rolled tagged union: one field is the tag, two others are "unspecified when null" | `EntityInland { archetype_ptr: *mut Archetype, unit_index, generation }` | `Option<LiveEntity>` — same layout and offsets; layout-free, ~2 register ops in a micro-shape, measure at the site | ERG-04, ERG-22 |
| An `unsafe fn` whose contract is "the caller registered it", re-stated at every caller | `get_layout_unchecked(component_id: usize)` and every SAFETY clause above its callers | a proof-carrying id minted only by registration and a SAFE `[]` — the `unsafe fn` and its contract go; the bounds check stays (no measured time on sequential shapes, and a forged id panics rather than reading the wrong row); no mask | ERG-03 |
| A sink that is written, whose consumers are fixed by the builder's push order, with reachability asserted in a comment | `injector_local[wid]` — `push_task` writes it, `stealers` never lists it, two docs said siblings reach it "via stage 1.5" (no such stage): every `par_iter` in a system body at 1.01×; `scope.rs`'s `scratch: Worker::new_fifo()` with no `Stealer`: 4–5 of 16 tasks in flight | a handle minted ONLY by the registration that puts the queue into a scan set; `push_task` names its destination by that handle, so an unreachable slot cannot be spelled and an unregistered deque reads as such — 0 diff lines | ERG-03 |
| Manual set / undo pairing around a body that can return early or panic | `depth += 1; body(); depth -= 1` | a `Drop` guard — the normal path is identical, the unwind path is what you bought | ERG-43 |
| `SeqCst` because it is "the safe one" | (the threadpool is already right: 1 `SeqCst` in 54 orderings, a documented fence) | the weakest ordering that discharges a named pairing; a `SeqCst` store is a bus lock, a `Release` store is a `mov` | ERG-45 |
| `if T::FLAG` that a later refactor can quietly turn into a runtime field read | the previous draft of this guide taught it | `if const { T::FLAG }` — the kernel's own spelling, 132 sites; the refactor is a compile error | ERG-31 |
| `clone()`, `RefCell` or a side `Vec` to get past "cannot borrow `*self` mutably twice" | (all three banned) | `mem::take` the field, work on the owned local, put it back — LLVM deletes the moves | ERG-43 |
| A layout claim in a doc comment | `Mat4` "uploads directly to a GPU uniform", no pin | a `const _` gate beside the type; `const { assert!() }` for a generic parameter | ERG-01 |
| A `#[derive]` on a generic tag type, which quietly bounds the parameter the type does not contain | `#[derive(Clone, Copy)]` on `ResState<R>` / `MutState<T>` / `WithState<C>` and twelve siblings, over `PhantomData<fn() -> R>` — and `res.rs`'s doc stating the false belief verbatim ("`Copy` is derived because the state carries only the `ResourceId`") | four hand impls, as `asset/handle.rs` already writes them ("hand-implemented (not `derive`) for the same reason") — an ICF alias, and `Copy` no longer conditional on the resource | ERG-47 |
| An id that indexes a table typed `usize`; a composite handle padded to twice its content | `define_id!` emits `pub struct $name(pub usize)` for nine ids; `pub type Generation = usize`; `Entity { EntityId, u32 }` = 16 B with 4 B of padding and `Option<Entity>` = 24 | `u32` per index, `Entity { u32, NonZeroU32 }` = 8 B under `repr(C, align(8))` with `to_bits` / `from_bits` — `Option<Entity>` free, `Ord` 4 instructions instead of 12, the probe 1.5–2.1× faster | ERG-02, ERG-04 |
| `ptr as usize` for a number, and `usize as *const T` back | `scope.rs`'s two `splitmix64(shared as usize as u64)` seeds; `component_pool.rs`'s alignment test; `tls.rs`'s address roundtrip in a Miri-covered test | `.addr()` for the number, `expose_provenance` / `with_exposed_provenance` for the roundtrip — four ICF aliases, and the roundtrip stops being UB under the model this repo's Miri runs | ERG-22 |
| An `unsafe` block whose only stated fact is that the callee is `#[target_feature]` — and prose saying that is required | `systems.rs`'s two blocks around the SAFE `sdf_edit_list_x8`, SAFETY reading "must be called from an `unsafe` block on stable"; `bitset_intersects_avx2`, an `unsafe fn` whose whole `# Safety` is "the caller must guarantee AVX2" | annotate the CALLER: a `#[target_feature]` island with ONE boundary `unsafe` whose sentence is the crate-wide `cfg` — 0 diff lines, and a whole class of blocks gone | ERG-46 |
| `#[repr(align(64))]` used for BOTH "aligned for a line" and "padded against the next line" | 39 hand-rolled `align(64)` sites in 20 files, 0 `align(128)`, beside `CachePadded` (128) in 7 — including `LogLane`, whose producer and consumer partitions are exactly 64 bytes apart | say which purpose: a SIMD/page alignment stays 64; a false-sharing pad is 128 on x86_64/aarch64 (`CachePadded`, already a dependency), and the site states the memory delta | ERG-01 |
| Two ALIGNMENT REGIMES over one pointer type, told apart only by which intrinsic the author typed | 125 `read_unaligned` / `write_unaligned` occurrences in 20 files (`command_queue.rs`, whose header names invariant CQ-PACK1 in prose and repeats it four times; `events/erased_buffer.rs`; `boyko_log::lane`) beside the aligned `read`/`write` off `ComponentPool` / `VmColumn` / `Column` row bases — the same `*mut u8` in both | `RowPtr<'a, A: IsAligned>` over sealed `Aligned` / `Unaligned` ZSTs, with `to_unaligned()` a one-way downgrade — 8 bytes, 0 diff lines in three pairs, and the wrong `read::<T>()` is `E0308` | ERG-48 |
| A property of an index STREAM — "no other worker writes this index" — restated in every SAFETY block that touches the stream | `ScratchSolveView::row_ptr`'s `# Safety`, 84 call sites in 19 files; `colored.rs`'s 50 SAFETY blocks, of which 10 restate the C2 coloring invariant, plus the two `unsafe impl Send`/`Sync` whose whole justification it is | an empty sealed `unsafe trait DistinctIndexWrites` as the BOUND — ICF alias at 16 and 1 codegen units; a type without the impl is `E0277`; each SAFETY block keeps its in-bounds clause and loses the other | ERG-48 |
| A device-owned resource freed by a call whose only precondition is a sentence | 118 "fence-waited" / `wait_idle` preconditions across 23 files of `boyko_rhi_vulkan` and `boyko_render`; `accel.rs`'s `destroy_accel` "pending (caller: fence-waited or `wait_idle`'d)" | `submit(t) -> Pending<T>` and `reclaim(Pending<T>) -> T` — ownership returns through a CALL, `destroy` is unreachable without it, 14 = 14 instructions (`mem::forget` still compiles, which is why the payload is `ManuallyDrop`) | ERG-48 |
| `#[repr(C)]` on a type that claims no layout | 230 `#[repr(C` sites in 71 files under `boyko_ecs`; 118 inside `#[cfg(test)]`, 82 with no gate and no `offset_of!` within 20 lines — including four single-`u32` fixtures and two ZST tags in `query/state.rs` | delete it where nothing claims a layout: `{u8,u64,u8,u32}` is 16 bytes as `repr(Rust)` and 24 as `repr(C)`, and the 8 bytes never appear in a diff | ERG-01 |

The paragraph this repository already wrote about the thesis, worth reading once —
`crates/boyko_log/src/codes.rs`, "# Class is a TYPE, not a field": *"A single `Code(u16)` with a
`class: u8` field would have made 'warn with a panic code' a runtime concern, and a runtime concern
about a diagnostic is one nobody sees until the diagnostic matters."*

## The MUST core — seventeen rules that are free AND mechanically checkable

Read these before writing code; the rest of the guide is a reference you grep when the shape comes
up. Every one was compiled in both spellings on this checkout: the surviving rules were reproduced
as zero-cost or negative-cost by an independent guard pass (EV-60), the new rule and the new
clauses by the editorial lab (EV-61 … EV-70) and the second revision's corrections by the same
lab (EV-71 … EV-73), each re-checked at the shipped codegen-units.

ERG-01 layout gate · ERG-02 domain newtype · ERG-04 sentinel is a type · ERG-05 `PhantomData`
row · ERG-06 token / scoped closure · ERG-07 no whole-buffer slice to a worker · ERG-12 no
`Box<dyn Iterator>` · ERG-14 closed sets are exhaustive enums · ERG-17 `Deref` only buffer→slice or
smart-pointer→value · ERG-20 `// SAFETY:` placement · ERG-26 `expect("invariant:")` /
`debug_assert!` · ERG-31 `if const { T::FLAG }` and the `compile_error!` pairs · ERG-39 names and
`bool`→enum · ERG-45 atomic orderings named · ERG-46 a target-feature `unsafe` is deleted, not
documented · ERG-47 a `#[derive]` on a generic tag type is hand-written · **ERG-48 an obligation
no representation can hold is a BOUND or a returned VALUE, not a repeated sentence**.

## Reading a rule

Each rule has a stable id (`ERG-nn`), a binding (**MUST** / **SHOULD** / **MAY** / **MUST-NOT** —
a MUST needs no judgement call; anything that does is SHOULD or MAY), one imperative line, a
BEFORE and AFTER from this tree wherever a site exists, what it buys, the cost verdict with the
method that established it, and the exceptions. Cost vocabulary: **ZERO-COST** — an
identical-code-folding alias, a zero-line normalised asm diff, a layout gate, an allocation count
or a bench on this checkout; **COMPILE-TIME ONLY** / **TEST-ONLY** — no runtime component;
**COSTS** — measured or definitional, and therefore refused or confined. Guarantee level says
whether the verdict rests on the language (Reference / std / RFC), on what rustc 1.97.1 happens to
do, or on a measurement at a site.

## What a new type owes

- **Always:** the newtype or enum itself, `///` saying why (ERG-02, ERG-41).
- **When a property is relied on:** the layout gate (ERG-01), the niche (ERG-04), the trybuild
  case for a "does not compile" claim that a later impl, derive or method could silently undo
  (ERG-10) — a plain transposition `E0308` between two distinct nominal newtypes owes NO
  fixture (REF-42).
- **On demand:** the macro when siblings must not diverge (ERG-02), operators when an operator is
  used (ERG-39), `#[must_use]` when dropping it is a bug (ERG-11).

## Layers — what a rule's scope words mean

Rules scoped by a layer name (ERG-14, ERG-15, ERG-28, REF-00, REF-11, REF-20, REF-24) use this
table, not a judgement. A function is classified by the innermost layer that CALLS it; when in
doubt it is kernel.

| Layer | What runs there | Modules |
|---|---|---|
| **Kernel** — per element, per frame | query iteration, storage access, solver inner loops, GPU column mirrors, command recording, SIMD math, the worker loop | `boyko_ecs::ecs::memory`; `boyko_ecs::ecs::core::{iters, archetype, component, entity, ecs_master, change_detection, events, resources}`; all of `crates/boyko_threadpool/src`; `boyko_physics::{solver, narrowphase, soft, systems}`; `boyko_math`, `boyko_sdf_math`, `boyko_utils`; `boyko_render`'s `*_system.rs` / `gpu_*` / `light_*` / `particle_*`; `boyko_rhi_vulkan::{present, compute, framegraph}` recording paths; `boyko_ui::layout`; `boyko_input::raw` |
| **Schedule** — once per system per frame | dispatch, run-conditions, set ordering, the dispatcher-solo window | `boyko_ecs::ecs::core::{schedule, system, state, time}`; `boyko_app::runner` |
| **Lifecycle** — when the program asks | spawn / despawn / clone / hooks / observers / commands apply; the erased per-type bodies | `boyko_ecs::ecs::core::{clone, commands, bundle, hierarchy, relationship}`; the `component_registry` tables |
| **Boot / load / tools** — once per process or per asset | builders, plugins, loaders, device creation, parse and report, serialize, diagnostics rendering, benches, tests | `boyko_ecs::ecs::core::{app, asset, serialize}`; `boyko_app` (except `runner`); `boyko_render::loaders`; `boyko_image`; `boyko_fontbake`; `boyko_serialize`; `boyko_rhi::device` `create_*`; `boyko_rhi_vulkan::{device, swapchain, window, memory}` setup; `boyko_ui::text`; `boyko_log` rendering; `boyko_demo`; `bench_*`; `examples/`; `tests/` |
| **External** — a type named in the published API | the mdBook / rustdoc surface | a LIST: the error enums `EcsError`, `AssetError`, `ScheduleBuildError`, `DecodeError`, `LoadWriteError`, `boyko_serialize::{SaveError, LoadError}`, `boyko_image::Error`; and `KeyCode`. `Access` and `LoadEntityPolicy` carry `#[non_exhaustive]` and are NOT on this row — report-only violations of ERG-14 |

## Accepted erasure — the `Box<dyn>` / fn-pointer / per-task allocation list

ERG-14 forbids a `dyn` or fn-pointer call PER ELEMENT on the kernel and schedule rows. The tree
erases at a handful of sites whose amortisation unit is a system, a task or a panic; they are a
LIST, and "per element" cannot be written in the unit column.

| Site | Layer | Amortisation unit |
|---|---|---|
| `SystemBox::system: Box<dyn System<Out = ()>>` — `schedule/system_box.rs` | schedule | one indirect call per SYSTEM per frame; boxed at schedule build |
| `BoolSystem = Box<dyn System<Out = bool>>` — same file | schedule | one call per CONDITION evaluation per frame |
| `TaskHandle::body: Box<dyn FnOnce() + Send>` — `thread_pool.rs`, boxed in `Scope::spawn` | kernel (threadpool) | one `Box::new` + one call per TASK (a `par_chunk` CHUNK, a spawned closure, an installed body — never a row). **Accepted AS MEASURED:** this is the cost KE16's variants report; a change is a KE16 decision with the campaign's own numbers |
| `Box<ScopeShared>` per `Scope` — `thread_pool.rs` | kernel (threadpool) | one allocation per parallel PHASE |
| `ScopeShared::panic_payload: AtomicPtr<Box<dyn Any + Send>>` — `scope.rs` | kernel (threadpool) | one box per CAPTURED PANIC (`#[cold]`) |
| `CloneFn` / `DropFn` / `MapEntitiesFn` / hook tables — `component_registry` | lifecycle | one fn-pointer call per ELEMENT, lifecycle only, with the trivially-copyable bypass (ERG-14 clause 3) |

A `Box<dyn>` on the kernel or schedule row that is not here is a violation to REPORT (orchestrator
and `docs/OPEN-QUESTIONS.md`), not a thing to register.

## Rules that yield to a principle, explicitly

| Rule | Yields to | At |
|---|---|---|
| ERG-08 typestate | Principle 3 (I-cache) | N states × M methods; stop at two or three states |
| ERG-15 shell / inner split | Principles 2 / 6 | never de-specialise a hot generic kernel — the monomorphisation IS the optimisation |
| ERG-39 operator set | `boyko_math` bit-determinism (principle 6, shader oracle) | `Div` is `x / s`, never `x * s.recip()`; no `mul_add` |
| ERG-28 inlining | Principle 7 | `#[inline(always)]` only with the measurement line |
| ERG-26 assertions | Principle 8 | a release `assert!` guards an `unsafe` write from caller data |
| ERG-35 adaptors | Principle 6 | a reduction that must vectorise is written lane-wise |
| ERG-36 `FromIterator` | Principle 0 | a fresh collection would be a parallel data system |
| ERG-12 / 14 / 15 / 17 / 20 | Principle 1 | restatements of the hot-path ban in signature form |

## The rules

### §1 Types and invariants — [rust-ergonomics/01-types-and-invariants.md](rust-ergonomics/01-types-and-invariants.md)

| ID | Binding | Rule |
|---|---|---|
| ERG-01 | MUST | A layout claim is a `const _` gate beside the type; a claim about a generic parameter is `const { assert!() }` in the monomorphised body. |
| ERG-02 | MUST / SHOULD | A domain integer that is stored, paired with a same-typed operand or used as an index is a `#[repr(transparent)]` newtype (a lone count with its noun in the name stays bare); a `[lo, hi)` pair is a `Range<T>`; siblings that must not diverge come from one macro. |
| ERG-03 | SHOULD | A bounded value is minted once into a private-field newtype and holding one IS the proof — of exactly the bound the mint establishes; a sink whose consumers are fixed at construction is written through a handle only the registration mints; the hot path indexes with a plain `[]` — `get_unchecked` only under a SAFETY naming the mint, with a measurement at the site; no range-narrowing mask. |
| ERG-04 | MUST / SHOULD | A sentinel is a type: absence is a `NonZero` niche; a mode is a data-carrying enum with a right-sized payload, or a stored transparent newtype with a view enum only when the payload needs its full width; never a magic constant compared by hand. |
| ERG-05 | MUST | `PhantomData` is chosen from the Nomicon table; `!Send` is pinned explicitly. |
| ERG-06 | MUST | A dangerous borrow is a non-`Copy` token minted once, or the argument of a caller-supplied closure — never a handle with a free lifetime. |
| ERG-07 | MUST | A worker never receives a whole-buffer `&mut [T]`: chunk slices, or a solve view with no slice surface. |
| ERG-08 | SHOULD / MUST-NOT | A two-phase protocol or builder mode is a ZST typestate — small payloads only. |
| ERG-09 | *merged* | into ERG-20 clause 5 (sealing an `unsafe`-contract trait) — second sweep. |
| ERG-10 | MUST | Every "no longer compiles" claim that an impl, derive or method could silently undo ships a trybuild case; a nominal `E0308` owes none. |
| ERG-47 | MUST | A `#[derive]` on a generic type emits a BOUND on the parameter; where the parameter is carried only in a `PhantomData`, the impls are hand-written and a `const _` gate pins that the bound is gone. |

### §2 API shape, naming and documentation — [rust-ergonomics/02-api-shape.md](rust-ergonomics/02-api-shape.md)

| ID | Binding | Rule |
|---|---|---|
| ERG-11 | MUST / MAY | `#[must_use]` with a message on guard / token / builder TYPES and on any status a caller may not drop; pure methods only where ignoring the value is a plausible bug. |
| ERG-12 | MUST | Return `impl Iterator` or a named type, never `Box<dyn Iterator>`; name what the kernel stores. |
| ERG-14 | MUST | A closed set is an exhaustive `#[repr(uN)]` enum: tag dispatch, no `dyn` per element on kernel / schedule (accepted-erasure list), integer-constant sets become enums at the API surface, `#[non_exhaustive]` only on the External list. |
| ERG-15 | MUST / SHOULD | Kernel signatures are concrete; conversion sugar lives at the boot boundary as a thin shell over one concrete inner `fn`. |
| ERG-17 | MUST-NOT | `Deref` only buffer→slice or smart-pointer→its one value (side effects documented); never on a domain newtype, proof type or solve view. |
| ERG-39 | MUST / SHOULD | Names and operators say what a call costs and means; a positional `bool` the name does not carry is an enum; math / flag types implement the full operator set under bit-determinism. |
| ERG-41 | MUST / SHOULD | A `pub` item's doc says why and names the decision; a suppression is `#[expect(lint, reason)]` and retires itself. |

### §3 Ownership, unsafe, pointers and failure paths — [rust-ergonomics/03-unsafe-and-pointers.md](rust-ergonomics/03-unsafe-and-pointers.md)

| ID | Binding | Rule |
|---|---|---|
| ERG-20 | MUST / SHOULD | `// SAFETY:` last before every block naming who establishes each fact; `# Safety` + forwarding on every `unsafe fn`; one obligation per block; raw base accessors `pub(crate)`. |
| ERG-22 | MUST | A raw pointer is minted with `&raw`, owned as `NonNull` with a variance marker, nullable as `Option<NonNull>` — never a null sentinel tested by hand. |
| ERG-24 | MUST | Slice API before raw-pointer arithmetic; a raw pointer only for a named aliasing reason with its id, or with a measurement at the site. |
| ERG-43 | MUST / SHOULD / MAY | An undo is a `Drop` guard; a borrow conflict is `mem::take`, never a clone or side store; copied-out bytes are `ManuallyDrop`. |
| ERG-45 | MUST | An atomic's ordering is the weakest that discharges a named happens-before, and the site names its pairing; `SeqCst` is never a default. |
| ERG-26 | MUST | `expect("invariant: …")`, never `unwrap()`; `debug_assert!` by default; a release `assert!` only against silent corruption, with that sentence at the site. |
| ERG-28 | MUST / SHOULD | Failure paths are cold `-> !` helpers; kernel errors are `Copy`; a FAILURE signal is `Result<(), Fieldless>`, never `bool`, and a predicate stays `bool`; `#[inline(always)]` and `select_unpredictable` only with the measurement line — the latter on BOTH predicate distributions. |
| ERG-46 | MUST / SHOULD | An `unsafe` whose only obligation is a target feature is DELETED: the caller carries `#[target_feature]`, the kernel stays a safe fn, and the island has exactly one boundary `unsafe` naming the crate-wide `cfg`. |
| ERG-48 | MUST / MAY | An obligation no REPRESENTATION can hold is a bound or a returned value, never a sentence repeated at every site: an alignment regime is a sealed type parameter; a property of an index STREAM is an empty sealed `unsafe` marker trait; a device-owned resource returns through a consuming `reclaim`; a "must give it back" is a `#[must_use]` ticket whose `Drop` tripwire is debug-only. |

### §5 Const evaluation and `cfg` — [rust-ergonomics/05-macros-and-const-eval.md](rust-ergonomics/05-macros-and-const-eval.md)

| ID | Binding | Rule |
|---|---|---|
| ERG-29 | *merged* | into ERG-31 clause 1 — second sweep. |
| ERG-33 | *merged* | into ERG-31 clause 4 (`compile_error!` pairs, axis banners, the build-variant witness, `cfg`-selected shims) — third sweep. |
| ERG-31 | MUST / SHOULD | Compile-time evaluation and configuration, four clauses: what can be `const` is `const` (POD `const DEFAULT`, `const fn` accessors, never an allocating `..Default::default()`); a per-type PREDICATE is an associated `const` under `if const { T::FLAG }`; a per-type POLICY that must bound a generic is an associated TYPE over sealed marker ZSTs, so the illegal pair stops being representable; every illegal `cfg` / feature pair is a `compile_error!`, every conditional site carries its axis banner, and a test-only substitution is a `cfg`-selected module of `pub use` re-exports. |

### §6 Iterators and data flow — [rust-ergonomics/06-iterators-and-data-flow.md](rust-ergonomics/06-iterators-and-data-flow.md)

| ID | Binding | Rule |
|---|---|---|
| ERG-35 | SHOULD / MUST-NOT | Adaptors by default; `zip` for paired columns with a hoisted release assert on caller lengths; no `chain` per element; identity verified per site. |
| ERG-36 | MUST / SHOULD / MUST-NOT | Honest `size_hint`; a release check under any `unsafe` write sized by `len()`; `IntoIterator for &Query` with the read-only bound; `Extend` on reserved capacity; never `FromIterator`. |
| ERG-38 | SHOULD / MUST | let-else, let-chains, `matches!`, labelled blocks; `ControlFlow` + `?` instead of a flag in closure-shaped bodies. |

## Refused — [rust-ergonomics/08-refused.md](rust-ergonomics/08-refused.md)

**Part A (costs):** REF-00 the un-linted hot-path ban · REF-01 `Deref` on a newtype · REF-02
`Box<dyn State>` typestate · REF-03 internal `#[non_exhaustive]` · REF-04 `#[must_use]` as
enforcement · REF-05 GhostCell · REF-06 stored `impl Trait` / `Box<dyn Iterator>` · REF-07
const-generic flags and math · REF-08 aliases as safety · REF-09 `pub` id fields · REF-10
proc-macro crates for a `macro_rules!` shape · REF-11 `#[track_caller]` · REF-12 bare
`PhantomData<T>` · REF-13 allocating `..Default::default()` · REF-14 `chain` per element · REF-15
`Index` on a proven path (no `unsafe` licence) · REF-16 `#[inline(always)]` unmeasured · REF-17
`FromIterator` · REF-18 reflexive `Debug` · REF-19 raw-pointer loops (review budget, not speed) ·
REF-20 `String` in kernel signatures · REF-21 "hoist config" (not a rule) · REF-22 lending
iterators · REF-23 a vacuous `!Send` assertion · REF-24 `thiserror` / `anyhow` · REF-25 `Vec` in a
`Resource` as bulk state · REF-26 blanket impls · REF-27 ungated `transmute` · REF-28 `OnceLock`
per element · REF-40 a range-narrowing mask on a proof-carrying id (a panic turned into a silent
wrong read, for no measured time) · REF-43 `core::hint::cold_path()` where the slow arm is already
a `#[cold]` fn (0 diff lines) · REF-44 a HOISTED `assert_unchecked` on a gathered-index maximum
(13 panic sites survive and the body grows 42 %) · REF-47 "push ifs up and fors down" at its named
site (the compacted form is 171 instructions / 104 vector ops against 127 / 113, and 0.96–1.03× at
every penetrating-corner ratio).

**Part B (ceremony):** REF-29 a newtype inside one function · REF-30 a one-implementor trait /
one-instantiation generic · REF-31 a builder for a POD · REF-32 a guard, marker or typestate for
what the boundary already guarantees · REF-33 an enum over a named-setter `bool` · REF-34 a
FULL-WIDTH data-carrying enum as a same-width sentinel's STORED form · REF-35 a container over newtyped elements;
units over `boyko_math` · REF-36 slice patterns as a perf claim · REF-37 `SeqCst` by default ·
REF-38 `#[must_use]` sprayed on pure methods · REF-39 a macro forced by counting to three ·
REF-41 a newtype over a lone count whose noun the method name carries · REF-42 a trybuild fixture
for a nominal `E0308` the language cannot lose · REF-45 `ParamSet` as a rule of THIS guide
(verified free — 110 vs 121 instructions, `E0499` — but unbuilt scheduler feature work, with no
author on record hitting the rejection it would remove) · REF-46 the leak-on-panic guard for an
in-place column rewrite (no site: two of three candidate `collect()`s are `#[cfg(test)]` and the
third builds a fresh id list) · REF-48 `Fn` instead of `FnMut` as a capture restriction (28 `FnMut`
bounds censused, essentially all `for_each`-shaped with a mutating caller) · REF-49 nested-tuple
folding in a derive (none of `boyko_macros`' eight derives can hit an arity ceiling).

## Checklist — run over the diff before submitting, in both directions

**Did I write C?**
- [ ] A bare `u32` / `usize` that is STORED, one of two same-typed positional integers in one
  signature, or used to INDEX storage; a `(lo, hi)` pair where `Range<T>` is the type; a `.get()`
  at a seam? (ERG-02)
- [ ] A magic constant standing in for a variant; an `Option<usize>`; an `X::MAX` compared by hand;
  one fact stored in two integer encodings kept in step by comments? (ERG-04)
- [ ] A `Copy` handle with a free-lifetime accessor and a "must not use after this line" comment?
  (ERG-06)
- [ ] A `bool` parameter whose noun is not in the method name; a `bool` return that means two
  things; a `bool` for a FAILURE — `false` is abnormal, or the callee already changed state —
  where only a predicate may stay `bool`? (ERG-39, ERG-28)
- [ ] A closed set as integer constants; a `match` with a `_` arm over an internal enum? (ERG-14)
- [ ] A null-sentinel raw pointer checked by hand; a `*mut T` field with no variance marker;
  `&x as *const _` where a reference must not be minted? (ERG-22)
- [ ] An invariant in a comment or a debug-only flag that a type could carry; an `unsafe fn` whose
  contract a proof-carrying id would discharge? (ERG-01, ERG-03, ERG-08)
- [ ] A queue, slot or buffer whose reachability is decided by the builder's push order and
  asserted in a comment; a push whose destination is a bare index? (ERG-03)
- [ ] A manual set / undo pair around a body that can return early or panic; a `clone()` /
  `RefCell` / side `Vec` to split a borrow? (ERG-43)
- [ ] `SeqCst` without a store–load argument; an atomic whose pairing is not named at the site?
  (ERG-45)
- [ ] `if T::FLAG` where `if const { T::FLAG }` would refuse the runtime refactor? (ERG-31)
- [ ] A raw-pointer loop where `split_at_mut` / `chunks_exact_mut` expresses the access? (ERG-24)
- [ ] A `#[derive]` on a generic type whose parameter is carried only in a `PhantomData` — so
  `Copy` / `Eq` / `Hash` is conditional on a parameter the type does not contain? (ERG-47)
- [ ] A `usize` id that indexes a table; a composite handle whose padding is as wide as a field?
  (ERG-02 clause 5, ERG-04 clause 4)
- [ ] `ptr as usize` for a hash seed or an alignment test; `usize as *const T` for a roundtrip?
  (ERG-22 clause 4)
- [ ] An `unsafe` block whose SAFETY says only "the callee is `#[target_feature]`"; an `unsafe fn`
  whose whole `# Safety` is "the CPU supports X"? (ERG-46)
- [ ] An `#[repr(align(64))]` that is meant to stop false sharing rather than to align a load —
  and therefore states the wrong number? (ERG-01 clause 4)
- [ ] A `#[repr(C)]` type uploaded to a device with a size gate but no no-padding proof; an FFI
  create-info holding a borrowed pointer with no lifetime? (ERG-01 clause 5, ERG-06 clause 3)
- [ ] A SAFETY clause repeated at more than a couple of sites because it is a property of a
  STREAM, of a buffer's HISTORY or of an external agent's TIMELINE — "these indices are distinct",
  "these bytes are aligned for `T`", "the device is still writing here"? (ERG-48)
- [ ] A `*mut u8` read with `read_unaligned` in one module and `read` in another, with nothing in
  either type saying which? (ERG-48 shape 1)
- [ ] A `destroy` / `free` / `unmap` whose `# Safety` says the caller fence-waited? (ERG-48 shape 3)
- [ ] `#[repr(C)]` on a type that claims no layout — a test fixture, a ZST tag, an internal struct
  nobody byte-views? (ERG-01 clause 6)
- [ ] An `include_bytes!` blob cast to `&[T]`, or a loaded id used without a range check?
  (ERG-01 clause 7)
- [ ] An `X::MAX` sentinel where `Option<NonMaxU32>` keeps index 0 meaning 0? (ERG-04 clause 5)

**Did I pile on abstraction?**
- [ ] A newtype for a value that never leaves one function; a trait with one implementor; a
  generic with one instantiation; a builder for a POD? (REF-29, REF-30, REF-31)
- [ ] A newtype over a lone count whose noun is in the method name; a proof type whose doc claims
  a bound its mint does not establish; a trybuild fixture for an `E0308` the language cannot
  lose? (REF-41, ERG-03, REF-42)
- [ ] A guard, marker or typestate for a property the boundary already guarantees; a typestate
  over an optional field or more than three states; a large payload consumed per transition?
  (REF-32, ERG-08)
- [ ] An enum over a builder-setter `bool`; a FULL-WIDTH data-carrying enum as a sentinel's stored
  form (ask first whether the payload needs all its bits); a container where newtyping the
  elements suffices? (REF-33, REF-34, REF-35)
- [ ] A range-narrowing mask or modulo on a proof-carrying id to delete a bounds check? (REF-40)
- [ ] `Box<dyn Iterator>`, `dyn` per element, `impl Into<String>` below boot, `#[non_exhaustive]`
  off the External list, `Deref` on a newtype, a blanket impl? (ERG-12, ERG-14, ERG-15, ERG-17,
  REF-26)
- [ ] `#[inline(always)]`, `get_unchecked`, `select_unpredictable` or `cold_path` without a
  measurement line at the site; a `#[must_use]` sweep; a macro forced by a count? (ERG-28, ERG-03,
  REF-38, REF-39, REF-43)
- [ ] A HOISTED `assert_unchecked` claimed to delete a per-element bounds check it cannot reach?
  (REF-44)

**Did I keep the obligations?**
- [ ] Every size / align / niche I rely on has a gate; every "does not compile" claim has a
  trybuild case counted by the witness? (ERG-01, ERG-10)
- [ ] `// SAFETY:` last before each block, naming who establishes each fact; `# Safety` on each
  `unsafe fn`; raw base accessors `pub(crate)`? (ERG-20)
- [ ] Every clause of that comment labelled VALIDITY (name the type or gate — then delete it) or
  SAFETY (name the module whose privacy upholds it — if none, that is the defect)? (ERG-20
  clause 6)
- [ ] `expect("invariant: …")`; `debug_assert!` by default; each release `assert!` has the "a
  vanished check would silently …" sentence? (ERG-26)
- [ ] Guard / token / builder types and undroppable statuses carry `#[must_use = "…"]`? (ERG-11)
- [ ] `zip` on caller-supplied slices has its hoisted release `assert!`; no `chain` per element;
  `size_hint` is exact or `(0, None)`? (ERG-35, ERG-36)
- [ ] POD configs have `const DEFAULT`; a new feature has its `compile_error!` pairs and banners;
  every `pub` item has `///`; every new suppression is an `#[expect]` with a reason?
  (ERG-31 clauses 1 and 4, ERG-41)

## Retired ids — what was merged or cut, and why

| Old id | Fate | Where it went / why |
|---|---|---|
| ERG-13 | merged | into ERG-15 — the shell / inner split is the boot-boundary half of the same signature rule |
| ERG-16 | merged | into ERG-39 — operators are call-site legibility; its stated reason ("copy-back the optimiser must prove away") was refuted (EV-60) and replaced |
| ERG-18 | merged | into ERG-14 clause 4 — `#[non_exhaustive]` is the exhaustiveness rule's other half |
| ERG-19 | merged | into ERG-28 — hot / cold placement and inlining are one discipline |
| ERG-21 | merged | into ERG-20 — one obligation per block is a clause of the SAFETY rule |
| ERG-23 | merged | into ERG-22 — `NonNull` + marker + `Option<NonNull>`; its MAY (receiver shape) was a non-decision and is cut |
| ERG-25 | cut | "one type owns an invariant" restated encapsulation; its checkable clause (`pub(crate)` raw accessors) is in ERG-20, its atomic-wrapper clause is ERG-45 |
| ERG-27 | merged | into ERG-26 — the `expect` message rule and the assertion cases are one rule |
| ERG-30 | merged | into ERG-29 — `const fn` accessors and `const DEFAULT` are one "what can be const is const"; its wide-type mechanism was wrong (no copy — EV-60) and is corrected |
| ERG-32 | demoted + merged | into ERG-02 as a SHOULD — a MUST triggered by counting to three was the guide's clearest ceremony risk (REF-39) |
| ERG-34 | cut | its MUST-NOT half duplicated REF-07; its one load-bearing fact (the padded-size gate formula) is in ERG-01 |
| ERG-37 | merged | into ERG-36 — the `IntoIterator` pair, `Extend` and `FromIterator` are iterator contracts; the blanket "pair wherever `iter` exists" is demoted to SHOULD outside the query surface |
| ERG-40 | cut | restated a gate the build enforces mechanically (`crates/boyko_log/tests`: orphan, page and premature-emitter checks); the class-as-a-TYPE paragraph lives in the C-in-Rust section above |
| ERG-42 | merged | into ERG-41 — two of its three halves restated live gates (the registry script, the ignore census); the `#[expect]` half is the MUST that survives |
| new ERG-43 | added | ownership tools: `Drop` guard, `mem::take`, `ManuallyDrop` — the guide's one systematic blind spot (50+ `impl Drop`, 143 `mem::take` sites, two recorded bug fixes, no rule) |
| new ERG-45 | added | atomic orderings — the largest failure-B hole for a lock-free kernel (measured: `SeqCst` store is a locked `xchg`) |
| ERG-04, ERG-31 | rewritten | ERG-04 now covers the mode-sentinel case its exception used to license; ERG-31 teaches the kernel's own stronger `if const` spelling |
| ERG-03, ERG-24 | rewritten | the `get_unchecked` and `split_at_mut` speed claims did not reproduce (EV-60, EV-70); both rules now stand on a safe spelling and on review budget |
| ERG-03 (mask clause) | withdrawn, rev. 2 | `index(self) = self.0 & (N - 1)` did nothing on a valid id and turned a bounds panic into a silent wrong read on an invalid one — irreconcilable with ERG-26 case 2 — and rested on an instruction count the ledger's own rule refuses; recorded as REF-40, EV-67 annotated |
| ERG-04 (shape 2) | rewritten, rev. 2 | the confirming reader: the worker id fits a byte, so a `#[repr(u8)]` enum with a right-sized payload is 2 bytes and IS the stored form (EV-71); the transparent-plus-view split is confined to full-width payloads; the example no longer names a view accessor `get` or hands a lane out as a bare `u32` (ERG-39, ERG-02) |
| ERG-02 exception · ERG-06 Before · ERG-20 After · ERG-01 trigger | corrected, rev. 2 | `claim_one_idle` was exempted by name and is now the Before, compiled (EV-73); `SharedPtr::as_ref` was cited as a good forwarding block and is now ERG-06's Before (EV-72); `from_raw_parts` triggers ERG-01 only where element size or alignment is relied on; the second role encoding in `boyko_diag::lane` and `prev_labels: Option<(u32, u16)>` are named in ERG-04 and ERG-02 |
| ERG-03 (`WorkerId` bound) | corrected, rev. 3 | ERG-02 / ERG-04 minted `WorkerId` from the idle-word bit scan while ERG-03 said holding one discharged `< worker_count` — a proof type that proved less than the rule claimed. The type now states ONE bound (`< 64`, the fact both mints establish), the `[]` and the `debug_assert!` stay, and the reader's alternative — a second type for the registered id — is refused as REF-32 (EV-76) |
| ERG-02 (integer decider · `Range`) | added, rev. 3 | a three-part test parallel to ERG-39's `bool` decider, so `pre_len` / `num_threads(n)` / `with_capacity(ids)` / `grow_to(id)` stay bare (REF-41); a `[lo, hi)` pair of one domain is a `Range<T>` (EV-74; a SHOULD — a reversed range still compiles, recorded) |
| ERG-03 (shape 2) | added, rev. 3 | the reachable-sink shape: `injector_local[wid]` and `scope.rs`'s unregistered `scratch` — the two largest measured threadpool defects — become a handle minted only by registration (EV-75); direction only, the type is the KE16 developer's |
| ERG-10 · ERG-28 | scoped, rev. 3 | the trybuild MUST covers properties an impl, derive or method can silently undo, not a nominal `E0308` (REF-42); the `Result`-over-`bool` clause gains the failure-versus-predicate decider (`unpark_one_idle -> bool` stays; `try_place_on_idle_sibling -> Result<(), TaskHandle>` earns it) |
| ERG-09 | merged, 4th rev. | into ERG-20 clause 5 — a seal answers, from the implementor's side, the same question ERG-20 asks of a block: who may establish the unsafe obligation. Body, `BitInteger` Before, `plugins.rs` After, EV-11 and both exceptions carried whole. One of the two payments for the sweep's two new rules |
| ERG-29 | merged, 4th rev. | into ERG-31 clause 1 — the two were one rule wearing two ids ("make the const evaluator SEE the decision"), and ERG-29's `const fn` accessor exists so the value can reach ERG-31's call-site constant. Body, Befores, Afters, EV-28 / EV-45 / EV-50 / EV-60 and all three exceptions carried whole. The second payment |
| new ERG-46 | added, 4th rev. | the featured island: `target_feature_11` (stable 1.86) makes a whole CLASS of `unsafe` blocks deletable, and the guide had no rule that DELETES unsafe — every unsafe rule assumed the unsafe was necessary. Sited twice (`systems.rs`, `bitset_intersects.rs`), 0 diff lines (EV-77), and it retires a false sentence the tree currently ships |
| new ERG-47 | added, 4th rev. | derive-induced bounds: the guide's own thesis — an invariant stated in one place and undone on the line above — applied to a DEFAULT rather than to a comment. Fifteen sites, the correct form already in tree at one (`asset/handle.rs`), ICF alias (EV-78) |
| ERG-01 · 02 · 04 · 06 · 08 · 15 · 20 · 22 · 26 · 28 · 31 · 35 | clauses added, 4th rev. | twelve second-sweep findings adopted as fourteen clauses on existing rules rather than as new ids, so the rule count did not move: padding width and the no-padding proof (ERG-01, EV-81 / EV-82); `u32` index width and the typed container (ERG-02, EV-79 / EV-84); the 8-byte `Entity` and the intrusive free list (ERG-04, EV-79 / EV-93); the FFI descriptor's lifetime (ERG-06, EV-85); `add_with` (ERG-08, EV-86); the lane trait (ERG-15, EV-87); sealing (ERG-20, EV-11); strict provenance (ERG-22, EV-83); the feature-gated assertion tier and `MaybeDbg` (ERG-26, EV-88); `select_unpredictable` (ERG-28, EV-90); `&` over `&&` in a vectorising fold (ERG-35, EV-91) |
| ERG-03 (cost sentence) · REF-15 | scoped, rev. 3 | EV-60's "no measurable time" is limited to the sequential-index shapes it measured; the gathered-index SIMD kernel in `colored.rs` is named UNMEASURED and stays on `row_ptr` under ERG-07 / ERG-24 (a) (OPEN 9) |
| ERG-33 | merged, 5th rev. | into ERG-31 clause 4 — §5's own title already pairs const evaluation and `cfg`, and both rules are one thesis: make the compiler SEE the decision so a configuration nobody meant cannot exist at runtime. Rule, `Before`, the `boyko_threadpool` `After`, the what-it-buys paragraph and all three exceptions carried whole. **The payment for ERG-48** |
| new ERG-48 | added, 5th rev. | the class the third sweep found and the guide had no vocabulary for: an obligation NO REPRESENTATION can hold — a property of an index stream, of a buffer's history, or of an external agent's timeline. Every other rule puts its fact in a value's representation; this one puts it in a BOUND or a returned VALUE. Four shapes, four sites, four measurements (EV-96 … EV-99), the third of which records its own falsifier (`mem::forget` still compiles) |
| ERG-01 (clauses 6, 7) · ERG-04 (clause 5) · ERG-20 (clause 6) | clauses added, 5th rev. | the converse of the layout gate (no claim, no `#[repr(C)]` — 16 bytes against 24, EV-100) and the embedded blob (`AlignAs` plus a validating load, EV-101, with its vectorisation claim REFUTED and the clause narrowed to the safety property); the top-end niche (`NonMaxU32`, EV-102, which also CONFIRMS rather than refutes ERG-04's one-niche-per-type exception); and the validity / safety split as `// SAFETY:`'s stopping rule plus the module-as-TCB numbers (EV-103) |
| ERG-22 (clause 4, roundtrip half) | withdrawn, 5th rev. | the clause prescribed `expose_provenance` / `with_exposed_provenance` and said the benefit "was not measured". `RUST-FRONTIER.md`'s FR-13 had already REFUSED that pair on FV-12's Miri run — the `as`-cast and the exposed pair produce the same warnings and the same strict-mode error — and FV-12 counted ZERO int-to-ptr casts in boyko's production code, so the one site the clause named is inside `#[cfg(test)]`. Deleted rather than superseded; the `.addr()` half stands on EV-83 unchanged |
| ERG-03 (OPEN 9 exception) | updated, 4th rev. | a lab model of the gather (EV-92) measured the safe `[]` at 0.76–0.79× of `row_ptr` — FASTER, three runs of three — and refuted the hoisted `assert_unchecked` that was proposed as the compromise (REF-44). The item stays open because the lab kernel is scalar and L2-resident where the tree's is 8-wide; what changed is which direction the site measurement should test first |

## Open — what this pass could not verify

1. **Miri — CORRECTED, 2026-09-03.** This item said no Miri run backed ERG-22, and ERG-22
   clause 4 repeated it. `RUST-FRONTIER.md`'s **FV-12 IS that Miri run**, and its result went the
   other way: it refuted the clause's roundtrip half (FR-13), which is now withdrawn. What
   survives of the item: ERG-22's ALIASING half still rests on RFC 2582 and the repository's own
   Tree-Borrows fixes rather than on a run, and the run that would close it is
   `-Zmiri-strict-provenance` over `boyko_ecs`, which FV-12 measured as blocked only by
   `crossbeam_epoch`'s `AtomicUsize` and belongs on an idle machine.
2. ~~**AVX2.**~~ **CLOSED and CORRECTED, 2026-09-02.** This item said the shipped build was
   SSE2 and that nothing set `target-cpu`. `.cargo/config.toml` has set
   `-C target-cpu=x86-64-v3` on all three x86 targets since 2026-09-02 (owner ruling, FMA
   included). The ledger's method paragraph carried the same stale sentence and is corrected in
   place. What survives of the item: **every absolute ns/element figure in EV-01 … EV-76 was taken
   at SSE2** and is read as a ratio, not as a number for the shipped build; the identity results
   are unaffected. Not re-measured — the box is loaded.
3. **The `[profile.release]` question.** The shipped release is codegen-units 16 (the ledger's
   method paragraph is corrected). Adding `[profile.release] codegen-units = 1` would make the
   0 %-gate methodology hold for release builds too — an owner / orchestrator decision, not made here.
4. **Lint first waves** — require building the repository, not done while other agents hold
   `crates/`: `unsafe_op_in_unsafe_fn = "deny"` and `clippy::undocumented_unsafe_blocks = "warn"`
   in the workspace lint table (ERG-20); `clippy::unwrap_used` at warn (ERG-26); `#![warn(missing_docs)]`
   per crate and the 462-site `#[expect]` migration (ERG-41).
5. **The task box is accepted as measured.** `TaskHandle::body` is what KE16 measures; this guide
   carries no second number.
6. **The `ComponentPool` size gate on wasm32** pins a 64-bit literal (ERG-01).
7. **One finder verdict still transferred rather than compiled:** the `ColorOcc` accessor pair
   (an ERG-04 shape on `resources.rs`) — cut the asm for the real coloring loop before adopting;
   the pass is bit-identity-gated. (The `IdleSet` half is closed: EV-76 compiled the idle-set
   shift over a bounded `WorkerId` at 0 diff lines; the newtyped `claim_one_idle` is EV-73.)
8. **The 2-byte `ThreadRole` was compiled in a lab, not in the tree** (EV-71): the public
   `current_worker_id() -> u32` and the `pub` sentinel constants are consumed by `boyko_ecs` and
   `boyko_physics`, so the sweep is cross-crate; the lane derivation is exact for two of three
   roles and the third is saved, as `install` already documents.
9. **The gathered-index kernel — NARROWED, not closed (EV-92).** A lab model of
   `colored.rs`'s cohort gather (64-byte rows, 4096 cohorts × 16 rows, 256 KB column, `v3`) says
   three things. (i) The HOISTED `assert_unchecked` proposed for it does not work: 13 panic sites
   survive and the body grows 261 → 371 instructions — refused as REF-44. (ii) The per-lane
   promise and `get_unchecked` are the same thing with different spellings (109 vs 107
   instructions, 0 panic sites), already covered by ERG-03. (iii) The safe `[]` ran at
   **0.76–0.79× of `row_ptr`** — FASTER, three runs of three, despite its 13 panic sites — so
   "the slice form could not elide its panic branch" is not by itself a reason to keep `row_ptr`.
   ⚠️ The lab kernel is SCALAR and L2-resident; the tree's is 8-wide with different aliasing, and
   the timings were taken under load. What remains open is the measurement AT THE SITE; the
   direction to test first has flipped.
10. **The intrusive entity free list is sited and measured but not adopted (EV-93).**
   `enum Slot { Live { NonNull, u32, u32 }, Dead { Option<NonZeroU32>, u32 } }` is still 16
   bytes, so `entity_master.rs`'s `free_entity_ids: Vec<EntityId>` — a parallel data system by
   principle 0 — can live in the dead slots' own bytes, halving the reallocations of a despawn
   burst (28 → 14). `alloc_id` grows 37 → 56 instructions. An architecture decision for the
   entity-storage owner, not a rule. **A THIRD ARM was added by the third sweep (EV-104):** encode
   liveness in the generation's PARITY — a live handle's generation is always odd, a freed slot's
   always even — so `is_alive` is a single `cmpl` / `sete` and the pointer is never consulted:
   **3 instructions against 6 for both of the other two arms**, which themselves ICF-alias each
   other. Same 16 bytes. The price is one increment: `free_id` 3 → 2, but `alloc_id` 4 → 6. This
   collapses two liveness fields into one (ERG-04's own header complaint, "one fact stored twice in
   two encodings") and it is the same owner's decision, on the same slot.
11. **The false-sharing pad is a layout fact with no runtime number here (EV-81).** ERG-01 clause
   4 states the distinction and the memory delta; the benefit of widening a `LogLane`-shaped
   partition from 64 to 128 needs a third arm in
   `crates/boyko_log/benches/lane_padding_ablation.rs`, run on an IDLE machine. Until then no
   site widens.
12. **`cargo llvm-lines`, `cargo bloat` and `cargo mutants` are not installed on this box.**
   REF-07's "32 instantiations = 3× code" and REF-18's "+43 % build" rest on hand counts, and the
   ledger now says so rather than borrowing an instrument's authority. `cargo mutants` is the
   instrument that would test whether ERG-10's fixtures DISCRIMINATE — the repository's own
   recorded meta-defect, a gate that cannot fail — and nothing checks that today.
13. **`#![no_std]` on `boyko_utils` is feasible, priced, and not done (EV-105).** REF-00 is the
   guide's only refusal enforced purely by review — `clippy.toml` denies the map and lock types,
   but `Box<dyn Trait>`, `Vec::new()`, `format!` and `String::from` are left to a reviewer. In a
   crate that does not link `alloc`, four of those five are not NAMEABLE. Measured on a verbatim
   copy outside the repository: `bit_mask` and `identifiers` compile under `#![no_std]` with NO
   `alloc` after a purely mechanical `std::` to `core::` substitution on seven import lines; the
   only residual blocker is `type_intern`'s `std::sync::OnceLock` (4 sites; `core` has no `Sync`
   once-cell, and adding `extern crate alloc` does not help at all); the two `Vec`-backed sparse
   maps are the crate's only `alloc` users, and `sparse_map`'s `Vec<Option<usize>>` is already
   ERG-04's worst measured site (EV-04 / EV-59, 2.55x) — the crate split and the ERG-04 fix are
   one edit. Two crates already carry `#![no_std]` (`boyko_sdf_math`, `boyko_shaderdsl`), so the
   pattern is house-approved. This is ENGINE work with an owner, not a rule; it is the only
   proposal in three sweeps that would convert a CLAUDE.md principle into a LINK error rather than
   better prose.
14. **Two `RUST-FRONTIER.md` rows are measured but unwritten (EV-107).** This pass was scoped to
   `docs/rust-ergonomics/` and `docs/RUST-ERGONOMICS.md` and did not touch the frontier document,
   which is where both belong. **Pattern types** (`u32 is 0..=63`) compile on the pinned
   `nightly-x86_64-pc-windows-gnu 1.100.0` with `use core::pattern_type;` and give
   `Option<WorkerId>` = 4 AND `Option<Option<WorkerId>>` = 4 — a niche of arbitrary width, the one
   thing `NonZero` and `NonMax` cannot give — but rustc warns that the feature "is internal to the
   compiler or standard library", so it is a BLOCKED-ON-STABLE row at best.
   **`core::mem::TransmuteFrom`** compiles on the same nightly, accepts `Mat4Pod -> [u8; 64]` and
   rejects a padded `#[repr(C)]` with `E0277` — i.e. exactly the proof `#[derive(bytemuck::Pod)]`
   already gives on stable (EV-82, ERG-01 clause 5), so it is a REFUSE row with its stand-in
   already adopted. Neither is an ergonomics rule; both numbers are in the ledger so the rows can
   be written without re-running them.
15. **A rayon-style `join_context` / `FnContext::migrated()` is a KE16 question, deliberately not
   an ERG rule** (REF-45's reasoning). Read on this checkout, no build: `join_on_worker` drains by
   `ScopeShared::pending`, a counter — there is no per-task identity at the join point, so "was the
   second half stolen" is NOT already available, and exposing it would need a new field or atomic
   on the per-task path. That puts it squarely on the accepted-erasure list's `TaskHandle` row and
   under ERG-45, and KE16's own numbers decide it. Recorded here only so nobody opens an ERG rule
   for it.

## Files

- [docs/RUST-ERGONOMICS.md](RUST-ERGONOMICS.md) — this front door: goal, bars, the deciding
  question, the C-in-Rust list, the MUST core, layers, accepted erasure, rule map, refused map,
  checklist, retired ids, open items.
- [rust-ergonomics/01-types-and-invariants.md](rust-ergonomics/01-types-and-invariants.md) — ERG-01 … ERG-08, ERG-10, ERG-47 (ERG-09 merged into ERG-20)
- [rust-ergonomics/02-api-shape.md](rust-ergonomics/02-api-shape.md) — ERG-11, 12, 14, 15, 17, 39, 41 (absorbs the former §7)
- [rust-ergonomics/03-unsafe-and-pointers.md](rust-ergonomics/03-unsafe-and-pointers.md) — ERG-20, 22, 24, 43, 45, 26, 28, 46, 48 (absorbs the former §4 and ERG-09)
- [rust-ergonomics/05-macros-and-const-eval.md](rust-ergonomics/05-macros-and-const-eval.md) — ERG-31 (ERG-29 and ERG-33 both merged into it)
- [rust-ergonomics/06-iterators-and-data-flow.md](rust-ergonomics/06-iterators-and-data-flow.md) — ERG-35, 36, 38
- [rust-ergonomics/08-refused.md](rust-ergonomics/08-refused.md) — REF-00 … REF-49, in two parts (REF-40, REF-43, REF-44 and REF-47 are cost refusals; REF-41, REF-42, REF-45, REF-46, REF-48 and REF-49 are ceremony / scope / no-site refusals — ids are stable, not positional)
- [rust-ergonomics/EVIDENCE.md](rust-ergonomics/EVIDENCE.md) — EV-01 … EV-107; the only place a number lives
- [RUST-FRONTIER.md](RUST-FRONTIER.md) — the companion in the other direction: this guide is the rules for code written NOW on the pinned stable channel; that document is the standing ADOPT / REFUSE / BLOCKED-ON-STABLE verdict per recent-Rust feature (FR-nn, ledger FV-nn), including the trait-solver answer and the price of any nightly pin — consult it before proposing a feature that is not in this guide.
- The former `04-errors-panics-assertions.md` and `07-naming-and-documentation.md` are merged into
  §3 and §2 respectively and no longer exist.
