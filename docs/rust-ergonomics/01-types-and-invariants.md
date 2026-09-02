# §1 Types and invariants — ERG-01 … ERG-10

Part of [RUST-ERGONOMICS.md](../RUST-ERGONOMICS.md). Cost verdicts cite [EVIDENCE.md](EVIDENCE.md).
Code is cited by file path and item name, never by line number.

The theme: an invariant that lives in a type is checked by the compiler on every build; an
invariant that lives in a comment is checked by whoever happens to re-read it. This engine
already pays for the type-level form in most places (`Tick`, `DispatcherToken`,
`ScratchSolveView`, `define_id!`, `worker_lane_for`); the rules below say "do it in the places
that still write C" — and every one of them was compiled in both spellings first.

---

### ERG-01 — A layout claim is a `const _` gate beside the type; a claim about a generic parameter is a `const { assert!(…) }` in the monomorphised body

**Binding:** MUST · **Cost:** COMPILE-TIME ONLY (EV-03) · **Guarantee level:** language

**Rule.** The trigger is a LAYOUT CLAIM, listed mechanically: `#[repr(C)]`, `#[repr(align)]`,
`transmute`, a `from_raw_parts` whose element size or alignment is relied on (a non-`u8` element,
or a byte count turned into an element count), a GPU or FFI upload, a `#[cfg(debug_assertions)]`
field, or the sentence "`Option<X>` is free". A `from_raw_parts` over `u8` with a byte length —
`crates/boyko_ecs/src/ecs/core/component/dense/views.rs`, `DenseBuildView::as_mut_bytes`, builds
`&mut [u8]` from `len * stride` — makes no layout claim and gets no gate: a MUST that fires with
nothing to write is a MUST a developer learns to skim. Write `const _: () = assert!(…, "<decision
id>: <what must not drift>")` immediately below the type. When the property belongs to a GENERIC parameter (`C` must not be
bitset-stored; the tuple arity is ≤ 12), a `const _` item cannot name it on stable — put
`const { assert!(!C::STORAGE_IS_BITSET, "D4: …") }` at the top of a body that is monomorphised
over `C`, or `const { panic!("…") }` as the whole body of the overflow impl. The error fires at
the offending instantiation.

**Before** — `crates/boyko_math/src/mat.rs`, `Mat4`: `#[repr(C, align(16))]`, a doc saying it
"uploads directly to a GPU uniform", no pin. `boyko_math` is the one crate with `#[repr(C)]`
GPU-mirror types and zero gates.

**After** — `crates/boyko_ecs/src/ecs/memory/component_pool.rs`, `ComponentPool`:

```rust
#[cfg(not(miri))]
const _: () = assert!(size_of::<ComponentPool>() == 128,
    "Phase 4 IM-1: the vm->backing swap must NOT grow ComponentPool (host = 128 B)");
```

and the generic form, `crates/boyko_ecs/src/ecs/core/system/params/tuple_impl.rs`: the
13-parameter arity impl's body is `const { panic!("MAX_SYSTEM_PARAM_ARITY = 12") }`, so a
13-parameter system fails to BUILD instead of failing at first run. For `Mat4`:

```rust
const _: () = assert!(size_of::<Mat4>() == 64 && align_of::<Mat4>() == 16,
    "Mat4 is a std140 mat4x4; a field added here breaks the GPU mirror");
```

**What it buys.** A doc-comment layout claim rots the day a field is added and nothing goes red
until a shader reads garbage. The gate re-checks itself on every `cargo check`, and it catches the
author's own wrong beliefs: EV-03 failed on the guide's own `DspBuf<1>` sketch (16 bytes, not 9),
and the runtime-cost guard wrote `63 + 8` for `DspBuf<63>` and got `E0080` — the true padded size
is `N.next_multiple_of(align_of::<usize>()) + size_of::<usize>()` (EV-36).

**Verified.** EV-03: 38 gates compiled in the release profile emit no code; a wrong one is
`E0080`. Const evaluation precedes codegen, so the generic-body form emits nothing either.

**Exceptions.**
- A gate whose value depends on pointer width is written against `size_of::<usize>()` or is
  `#[cfg(target_pointer_width = "64")]`-gated. The `ComponentPool` gate above is not; `boyko_demo`'s
  wasm32 build compiles it with 4-byte pointers (OPEN 6). Report-only while `boyko_ecs` is under edit.
- A type with a `#[cfg(debug_assertions)]` field (ERG-26 case 3) has two sizes: gate each profile.
- A size gate proves size, not field order; when order matters, `repr(C)` plus `offset_of!`.
- The generic-body assert fires only for instantiations that are actually monomorphised — an
  impl nobody uses is never checked. Its message must name the decision id, because a
  `const { panic!() }` cannot format the offending type.
- A gate in a `#[cfg(test)]` module only fails `cargo test`; put it beside the struct.

---

### ERG-02 — A domain integer that is stored, paired with a same-typed operand, or used as an index crosses a boundary as a `#[repr(transparent)]` newtype; a half-open pair is a `Range<T>`; never an alias, never a bare `usize` at a storage seam

**Binding:** MUST (the newtype, by the three-part test below); SHOULD (`core::ops::Range<T>` for
a `[lo, hi)` pair of ONE domain; three or more siblings of one shape come from one `macro_rules!`
that emits the WHOLE shape) · **Cost:** ZERO-COST (EV-01, EV-59, EV-60, EV-73, EV-74) ·
**Guarantee level:** language (layout, ABI); measured (codegen)

**Rule.** A mechanical test, parallel to ERG-39's for a `bool`. The newtype is a MUST when the
integer
1. is STORED — a struct field that names a thing (`WorkerLane.wid`, `GpuColumnMeta.stride`); or
2. is one of two or more SAME-TYPED positional operands of one signature
   (`new(component_id, reserve_rows)`, `create_column(stride, cap_rows)`,
   `claim_one_idle(observed, exclude, ..)`) — the tell; or
3. INDEXES storage, or is compared against another domain or a sentinel (`inner.workers[id]`,
   `slots[i] == ABSENT`).

A LONE integer stays bare when the method name carries its noun AND it is consumed as a count or
a comparison — neither stored nor used as an index: `wake_after_push(inner, pre_len: usize)`
(compared with `> 1`), `injector_pre_len` / `deque_pre_len -> usize`,
`EntitySlotMap::with_capacity(ids: usize)` and `grow_to(id: usize)` (a `resize` length),
`ThreadPoolBuilder::num_threads(n)`, `manifold_fill(n: usize)`. A `QueueLen(usize)` there is a
name to learn for no confusion prevented (REF-41). `pub type Foo = usize;` is not a type. A
`.get()` written at a call site to feed a raw integer into a signature is the visible symptom of
a seam the type is not protecting. And a pair that is a half-open RANGE of ONE domain —
`(start, end)` — is not two domains and gets no per-domain newtype: it is `core::ops::Range<T>`
(`[lo, hi]` is `RangeInclusive<T>`), the std type whose fields say which is which and which a
slice accepts as an index.

**Before** — `crates/boyko_ecs/src/ecs/memory/component_pool.rs` and its callers:

```rust
pub fn new(component_id: usize, reserve_rows: usize) -> Self       // transposed call compiles
pub fn swap_remove(&mut self, index: usize) -> bool                 // row and id share a type
// caller, asset/assets.rs — the newtype is stripped exactly where it was protecting the call:
col: ComponentPool::new(component_id.get(), reserve_rows),
```

`crates/boyko_render/src/gpu_column.rs`, `create_column(.., stride: u32, cap_rows: u32)`: a
swap passes both `> 0` debug asserts, allocates the right number of bytes (multiplication
commutes) and writes `GpuColumnMeta { stride: cap_rows, device_cap: stride }` — a silently
corrupt device column. And `crates/boyko_ecs/src/ecs/identifiers/primitives.rs`: `pub type
Generation = usize;` twenty lines below `define_id!` minting nine real newtypes — while
`boyko_utils` defines `Generation = u32` and `Entity.generation` is `u32`, bridged by two
`From` impls where a width change is a silent truncation.

And in the file the developer is about to touch — `crates/boyko_threadpool/src/worker.rs`,
`claim_one_idle(inner, observed: u64, exclude: u64, start: u32) -> Option<u32>`: `observed` is
the CAS's EXPECTED operand ("must be the value the caller actually read, never a masked
derivative of it"), `exclude` is a bit mask, and the return is a bit position that
`unpark_one_idle_excluding` hands straight to `inner.workers[id as usize]`. Forty lines of doc
explain that folding or confusing the two makes the CAS unsatisfiable and spins forever.
`claim_one_idle(inner, exclude, observed, start)` compiles today, and so does
`claim_one_idle(inner, observed & !exclude, exclude, start)` — the exact call the doc says hangs.
`crates/boyko_threadpool/src/thread_pool.rs`, `InstallGuard.prev_labels: Option<(u32, u16)>`: a
worker id and a diagnostics lane as two bare integers (one `Option` over the pair is the right
shape — the integers are not; ERG-04).

`crates/boyko_physics/src/solver/colored.rs`: `solve_color(.., span: (usize, usize), ..)`,
`solve_color_dispatch(.., span: (usize, usize), g_lo: usize, g_hi: usize, ..)` — with a doc
sentence that `span` and `[g_lo, g_hi)` "MUST describe the same contiguous slot region" — and
`let (start, end) = span; for i in start..end { .. view.ra(i) .. }`. The tell fires (two
same-typed positional integers), but both members ARE slot indices: a `SlotIndex(usize)` newtype
would not touch the confusion this pair has, which is start-versus-end, and the pair cannot
index a slice.

**After** —

```rust
pub fn new(component_id: ComponentId, reserve_rows: RowCapacity) -> Self
pub fn swap_remove(&mut self, row: RowIndex) -> Result<(), RowOutOfBounds>   // ERG-28
pub fn create_column(.., stride: ByteStride, cap_rows: RowCount) -> Result<..>
#[repr(transparent)] #[derive(Clone, Copy, PartialEq, Eq, Hash)] pub struct Generation(u32);  // once, in boyko_utils

/// The idle word AS LOADED — the CAS's expected operand. Deliberately NO `BitAnd` / `BitOr`:
/// a masked derivative cannot be spelled as an `IdleWord` at all.
#[repr(transparent)] #[derive(Clone, Copy, PartialEq, Eq)] pub struct IdleWord(u64);
#[repr(transparent)] #[derive(Clone, Copy, PartialEq, Eq)] pub struct ExcludeMask(u64);
impl IdleWord {
    #[inline] pub const fn claimable(self, exclude: ExcludeMask) -> u64 { self.0 & !exclude.0 }
    #[inline] pub const fn without(self, id: WorkerId) -> u64 { self.0 & !id.bit() }   // `bit()` is never masked: the mint bounds `id < 64` (ERG-03)
}
pub(crate) fn claim_one_idle(inner: &PoolInner, observed: IdleWord, exclude: ExcludeMask, start: WorkerId) -> Option<WorkerId>
prev_labels: Option<ThreadLabels>,                                            // ERG-04

fn solve_color(view: ContactSolveView<'_>, bodies_eff: .., span: Range<usize>, ..)   // `for i in span`; `span.len()`, `span.is_empty()`
fn solve_color_dispatch(.., span: Range<usize>, groups: Range<usize>, ..)            // `[g_lo, g_hi)` is one value with named ends
```

**What it buys.** `ComponentPool::new(reserve_rows, component_id)`, `create_column(rows,
stride)` and `claim_one_idle(inner, exclude, observed, start)` become compile errors instead of
layout corruption or a hang — and so does the folded call, because the type that must not be
derived from has no operator that derives. Two same-typed positional integers in one signature
is the tell; the fix costs the reader one type name and the machine nothing. The range states
its order where it is built (`start..end`), `len()` saturates at zero where `end - start` wraps,
and `&xs[span]` becomes spellable — which hoists the per-element bounds check out of the loop
(EV-74). Honestly: a reversed range still compiles and iterates empty; that clause is legibility
and slice-reachability, which is why it is a SHOULD and not a MUST.

**Verified.** EV-01: a summing loop over `&[Id]` and `&[u32]` folded to one symbol. EV-59: the
in-tree finder's lab emitted `size_rust = size_c` (the `ByteStride × RowCount` product) and
`slot_eq_r = slot_eq_c` (the `Generation` newtype) as ICF aliases, and byte-identical bodies
for a two-domain CSR walk (124 instructions each, vectorised). EV-60: a 64-byte
`#[repr(transparent)]` wrapper passed BY VALUE across a non-inlined boundary aliased to the
plain form. EV-73: the newtyped `claim_one_idle` and the `u64, u64, u32` original folded to ONE
symbol (`claim_r32 = claim_c`) at codegen-units 1 and 16; with the byte-sized `WorkerId` of
ERG-04 the body is 18 instructions against 19 (byte arithmetic, one fewer `movl $0`) and a
caller loop over the two is 43 = 43; the transposed call and the folded call are both `E0308`.
EV-74 (this pass): `for i in start..end { xs[i] }` over the pair and `for i in span { xs[i] }`
over the `Range` are 17 instructions each, 0 normalised-diff lines beyond the panic constant, at
1 and 16 units; both types are 16 bytes, align 8; `for &x in &xs[span]` is 23 instructions with
the bounds check hoisted to two compares before the loop and none inside; `Range::len` is one
`cmovae` more than `end - start` and returns 0 for a reversed range where the subtraction
wraps. `f(end..start)` compiles and returns `len() == 0` — recorded so the clause is not read
as a refusal.

**Exceptions.**
- A value that never leaves one function body has no boundary to cross and gets no newtype (the
  deciding question in the index; REF-29); a lone count whose noun the method name carries stays
  bare even across a boundary (REF-41).
- Two same-typed operands of one signature are newtyped as a PAIR, never one of them: with only
  `ExcludeMask` typed, `claim_one_idle(inner, observed & !exclude, exclude, start)` still compiles.
  And the member that must not be derived from carries NO `BitAnd` / `BitOr` / `Not` — its only
  operations are the two the body needs — or the newtype merely renames the hazard.
- `Range<T>` is not `Copy` — it is an iterator. Pass it by value or `.clone()` it (two words,
  deleted like any small move). Where a `Copy` view must carry the bounds, keep a struct with
  NAMED fields (`Span { start, end }`), never a tuple.
- A field the OS, the GPU or a wire format reads keeps its integer in the STORED struct; the
  newtype lives at the API surface (`SdfEdit::op` stays `u32`; the constructor takes `SdfOp`).
- No `Deref` (ERG-17); no `pub` field if the type may ever carry a proof (ERG-03); `#[derive(Default)]`
  mints id `0` silently — derive it only when `0` is a valid id.
- The macro (`define_id!`, `code_newtype!`) is a SHOULD, triggered by "these siblings must not
  diverge", not by counting to three: a macro hop degrades goto-definition and cannot carry a
  per-site `// SAFETY:`. It must not emit a `pub` field, a `Deref`, or an unconditional `Default`.

---

### ERG-03 — A bounded value is minted once into a private-field newtype and holding one IS the proof — of exactly the bound the mint establishes; a sink whose consumers are fixed at construction is written through a handle only the registration mints; the hot path indexes with a plain `[]`

**Binding:** SHOULD · **Cost:** the `unsafe fn` and its contract are deleted for free; the bounds
check a plain `[]` keeps on a proof-carrying id is one compare and a never-taken branch with no
measurable time on the SEQUENTIAL-index shapes EV-60 measured — the gathered-index SIMD kernel
is unmeasured (Exceptions); the registration handle is 0 diff lines (EV-75); the mask that would
delete the check is REFUSED (REF-40) · **Guarantee level:** measured

**Rule.** Two shapes of one move — a fact established once, at construction, carried by a value
nobody else can make:
1. **A bounded id.** Where an `unsafe fn` exists because "the caller guarantees the id was
   registered / the index is in range", make the value a newtype whose ONLY constructors are the
   paths that establish the bound, keep the field private, and index with plain `[]`. The type
   carries EXACTLY the bound every constructor establishes and no more: a type with two mints
   proves the weaker of their two facts, and its doc says which. The check is paid once, where
   the value enters; the `unsafe fn`, its `# Safety` section and every downstream `// SAFETY:`
   clause that restated the precondition are deleted. The bounds check `[]` keeps is the
   compiler's, not yours, and it is the RIGHT failure: a forged, stale or wrongly-widened id
   PANICS instead of reading the wrong row — ERG-26 case 2, applied to the registry every storage
   path keys on.
2. **A reachable sink.** Where a queue, slot or buffer is written by one party and must be
   reachable by others, and "reachable" is a property of the CONSTRUCTION SEQUENCE (this deque's
   `Stealer` was pushed into the scan set; this slot is in every sibling's probe list), the
   destination a writer names is a handle minted ONLY by the registration that makes it
   reachable. A write into a slot no consumer scans then has no value to be spelled with, and an
   unregistered queue reads as such at its declaration — it has no handle.

**Before** — shape 1, `crates/boyko_ecs/src/ecs/core/component/component_registry/mod.rs`:

```rust
/// # Safety
/// Caller guarantees that `component_id < MAX_COMPONENTS` and that `register_new::<T>()` …
pub unsafe fn get_layout_unchecked(component_id: usize) -> &'static ComponentLayout
// component_pool.rs, and every transitive caller:
// SAFETY: component_id was checked above; caller must have registered the component …
let layout = unsafe { component_registry::get_layout_unchecked(component_id) };
```

`crates/boyko_threadpool/src/tls.rs`, `WorkerLane { pub(crate) wid: u32, .. }` with "Always `<
inner.worker_count()`" in the doc: the bound is proved once in `worker_lane_for` and then cast
back with `as usize` at every `inner.injector_local[..]` / `inner.workers[..]` index.
`crates/boyko_threadpool/src/worker.rs`, `mark_idle` / `unmark_idle`: `1u64 << worker_id` behind
`debug_assert!((worker_id as usize) < MAX_WORKERS)` — in release the assert is gone and `shlq`
masks its count to six bits, so an id of 70 marks worker 6 idle: a lost wakeup, a hang, no
diagnostic.

Shape 2 — the two largest measured threadpool defects, and they are one shape
(`docs/OPEN-QUESTIONS.md`, 2026-08-30). `crates/boyko_threadpool/src/thread_pool.rs`,
`ThreadPoolBuilder::build`, is where reachability is DECIDED: `stealers.push(w.stealer())` once
per deque, and `injector_local` built beside it with no stealer at all; `PoolInner::injector_local`'s
doc carries the consequence in capitals — "NOTHING ELSE DRAINS THAT SLOT … the sibling scan walks
`stealers` only". `push_task` sends a worker's spawn there, so every `par_iter` in a system body
ran at 1.01× while two doc comments asserted siblings reach it "via stage 1.5 of `worker_main`" —
a stage that did not exist (defect A). `crates/boyko_threadpool/src/scope.rs`,
`join_workers_until_drained`: `let scratch: Worker<TaskHandle> = Worker::new_fifo();` — a deque
with no registered `Stealer`, into which the joiner steals a batch of the wave and runs it
serially, so 4–5 of 16 tasks were ever in flight; the file header now names it ("KE16 defect B —
`scratch` is unregistered, so no sibling can take any of that batch back"). The invariant "this
sink has a consumer other than its producer" lived in the builder's push order and in two
comments, one of them false — fast, working, and wrong in a way nothing complained about.

**After** — shape 1:

```rust
/// The ONLY constructor is the registration path, so holding one IS the proof.
#[repr(transparent)] #[derive(Clone, Copy)]
pub struct RegisteredComponentId(u32);           // private field; minted by register_new only
impl RegisteredComponentId { #[inline] pub const fn index(self) -> usize { self.0 as usize } }
pub fn layout(id: RegisteredComponentId) -> &'static ComponentLayout { &LAYOUTS[id.index()] }  // SAFE; a forged id panics

/// `< MAX_WORKERS == 64` — the bound BOTH mints establish by construction: `worker_lane_for`
/// (which also checks the stronger `< worker_count()` and does NOT encode it here) and the
/// idle-word bit scan (`trailing_zeros` / `% 64`). `claim_one_idle`'s contract and its test say
/// bits above the worker count are legitimate inputs and outputs, so `inner.workers[id.index()]`
/// keeps its `[]` and `try_place_on_idle_sibling` keeps its `debug_assert!`: the type does not
/// discharge that obligation and does not claim to.
#[repr(transparent)] #[derive(Clone, Copy, PartialEq, Eq)]
pub struct WorkerId(u8);
impl WorkerId { #[inline] pub const fn bit(self) -> u64 { 1u64 << self.0 } }   // never masked: `self.0 < 64`
pub(crate) fn mark_idle(idle: &AtomicU64, w: WorkerId) { idle.fetch_or(w.bit(), Ordering::Release); }
pub(crate) struct WorkerLane { wid: WorkerId, .. }
```

Shape 2, as a DIRECTION — the concrete type is the KE16 developer's, not this guide's:

```rust
/// A queue that SOME thread's scan set polls. Private field: the only constructor is
/// `register`, which is also the only place a queue enters a scan set.
pub(crate) struct Scanned(usize);
impl ThreadPoolBuilder {
    fn register(&mut self, w: &Worker<TaskHandle>) -> Scanned { self.stealers.push(w.stealer()); Scanned(self.stealers.len() - 1) }
}
pub(crate) fn push_task(inner: &PoolInner, dest: Scanned, task: TaskHandle)      // a slot nobody scans cannot be named here
let scratch: Worker<TaskHandle> = Worker::new_fifo();                             // reads as unregistered: it has no `Scanned`
```

**What it buys.** Shape 1: an `unsafe fn` and its `# Safety` section leave the kernel; the
obligation moves from prose every caller must honour to a value no caller can fabricate; the
failure mode for a violated proof is a panic, never a silent wrong answer; and with the mint
bounding `WorkerId < 64`, the idle-set shift cannot mark the wrong worker. Shape 2: the purest
failure-B site in this tree — the code was fast and nothing complained — becomes a
construction-time fact the type checker sees, and the class KE16 exists to fix cannot be
reintroduced by a later push arm that names a bare index.

**Verified.** EV-67: the safe `[]` under the proof-carrying id is 13 instructions with `cmpl
$511; ja` and a cold panic path against 3 for `get_unchecked` — a count, which the ledger's own
reading rule says is not a cost. EV-60 (runtime-cost guard, three runs): removing a bounds
check under a proof changes the asm (22 → 48, the panic tail gone, LLVM unrolls) but NOT the
time — 0.765–0.791 vs 0.717–0.779 ns/elem on a 16 KB `f32` table, 0.793 vs 0.780 on a 256 KB
component column. EV-02 / EV-23's "1.5×" is a shape-specific result and no longer licenses
`unsafe`. EV-76 (this pass): `mark_idle` / `unmark_idle` over `WorkerId(u8)` are 0 diff lines
against the `u32` + `debug_assert!` form (6 instructions: `shlq %cl; lock orq`); the table
index `wake_r` keeps its `[]` compare, because the type proves `< 64`, not `< worker_count`.
EV-75 (this pass): a push through a `Scanned` handle and through a raw index are 20
instructions, 0 diff lines, at 1 and 16 units; constructing the handle outside its module is
`E0423`. EV-73: the newtyped `claim_one_idle` folds to the original. The previous revision's
`& (N - 1)` accessor is withdrawn (REF-40).

**Exceptions.**
- SHOULD, not MUST, because `define_id!`'s `pub` field (REF-09) forecloses the proof across the
  ECS; closing it is an architecture pass with a workspace-wide diff. Report it; do not do it
  opportunistically inside another change.
- **One bound per type.** A proof type that proves less than its doc says is worse than the bare
  integer it replaces, because the reader is told to trust it and delete the check. `WorkerId`
  proves `< 64`; a second type for "registered in THIS pool" (`< worker_count`) would turn
  `inner.workers[bit]` from a bounds panic into a compile error — refused (REF-32): in
  production every set idle bit IS a registered worker (only a worker marks its own bit), the
  one site that converts is `try_place_on_idle_sibling`, and `[]` already turns a violation into
  a panic rather than a wrong answer.
- **The cost sentence is scoped.** EV-60's "no measurable time" was taken on sequential-index
  shapes — a 16 KB `f32` table and a 256 KB component column, one proof-carrying id per element
  in order. `crates/boyko_physics/src/solver/colored.rs`, `solve_color_avx2`, reads sixteen
  `BodyEffective` rows per 8-wide cohort through `body_copy` / `body_mut` off GATHERED
  `body_a(i)` / `body_b(i)` indices, and its own comment records that the slice form "could not
  elide its panic branch" there. A per-lane bounds branch inside a gather is the one place a
  check is not known to be free; that kernel is UNMEASURED and stays on `row_ptr` under ERG-07
  shape 2 / ERG-24 case (a) unless a measurement at that site says otherwise (OPEN 9).
- There must be no `new_unchecked` without `unsafe`, no `pub fn from_raw`, and no
  `#[derive(Default)]`; `boyko_serialize` routes through the mint — a deserialised proof is a
  forged proof.
- `get_unchecked` under the proof is permitted only with the measurement at the site, exactly
  as `#[inline(always)]` is (ERG-28), and its `// SAFETY:` names the mint. No range-narrowing
  mask or modulo on the accessor (REF-40): it cannot be reconciled with ERG-26 case 2 at the
  same site.
- Shape 2's handle is a proof about the CONSTRUCTION, not about the queue's contents, and it
  says nothing about which thread may pop as OWNER — that is `worker_lane_for`'s question
  (ERG-06).

---

### ERG-04 — A sentinel is a type: absence is a `NonZero` niche; a mode is a data-carrying enum with a RIGHT-SIZED payload, or — only when the payload needs its full width — a stored transparent newtype with a view enum; never a magic constant compared by hand

**Binding:** MUST for new fields; SHOULD when retrofitting · **Cost:** ZERO-COST or smaller
(EV-04, EV-59, EV-71); a FULL-WIDTH payload in a data-carrying enum COSTS a word (EV-40,
EV-64) · **Guarantee level:** language (`Option` niche representation; RFC 2195 `#[repr(u8)]`
layout); measured (sizes, codegen)

**Rule.** Decided by two questions — how many states, and does the payload need all its bits:
1. **Two states (present / absent):** `Option<NonZeroU32>` (or a transparent newtype over it),
   storing `index + 1`; the bias lives in two accessors and is STATED AT THE FIELD, because the
   stored value no longer means what its name says. Gate `size_of::<Option<X>>() == 4`.
2. **Three or more states:**
   - **(a) The payload's domain is bounded below its integer's width** — a worker id is `<
     MAX_WORKERS == 64` inside a `u32` — so the stored form is a `#[repr(u8)]` data-carrying
     enum with the payload right-sized: `enum ThreadRole { Worker(WorkerId /* u8 */),
     Dispatcher, Unattached }` is 2 bytes, SMALLER than the sentinel it replaces, matched
     directly, with no encoding behind an accessor and no rule about which of two types may be
     stored.
   - **(b) The payload needs its full width** — an island id that may reach 2^24 in a `u32`, a
     value that is also a wire or GPU field — so the STORED type is `#[repr(transparent)]
     struct Stored(u32)` holding today's encoding, and the only way to read it is `fn role(self)
     -> View` returning a VIEW enum that is matched and discarded. The view is 8 bytes and stays
     a temporary.

**Before** — `crates/boyko_threadpool/src/tls.rs`: the worked example of C-in-Rust — and the
fact is stored TWICE.

```rust
pub const WORKER_ID_DISPATCHER: u32 = u32::MAX - 1;
pub const WORKER_ID_UNATTACHED: u32 = u32::MAX;
static CURRENT_WORKER_ID: Cell<u32> = const { Cell::new(WORKER_ID_UNATTACHED) };
// current_worker_id_or_dispatcher_lane — every consumer re-derives the three cases by hand:
if id == WORKER_ID_DISPATCHER { worker_count } else if id == WORKER_ID_UNATTACHED { 0 } else { id }
```

`worker_lane_for` asks the same question a THIRD way (`wid as usize >= inner.worker_count()`),
`tests/miri_scope.rs` spells the second sentinel as the literal `u32::MAX`, and
`tests/cross_pool_routing.rs` records that a predicate asking for the deque pointer alone "would
have indexed `inner.workers[WORKER_ID_DISPATCHER]` here and killed the process". The SAME fact
lives a second time in `crates/boyko_diag/src/lane.rs` as a `u16` with its own sentinel family —
`LANE_DISPATCHER = 64`, `LANE_HOST = 65`, `LANE_SPARE_BASE = 66`, `LANE_UNCLAIMED = u16::MAX` —
kept in agreement with the `u32` by a discipline written in comments: "D1 lane write site 1 of
3, deliberately co-located" (`worker.rs`, `worker_main`), "site 2 of 3" (`thread_pool.rs`,
`install`), "site 3 of 3, and the one an implementer working from the decision record's 'two
sites' misses" (`InstallGuard::drop`), with `prev_labels: Option<(u32, u16)>` carrying both as
bare integers and a doc explaining that a half-restore "is exactly the D1 defect this guard
exists to prevent". Same shape elsewhere:
`crates/boyko_ecs/src/ecs/core/component/dense/entity_slot_map.rs` (`ABSENT = u32::MAX`, nine
hand comparisons and a `debug_assert_ne!` whose only job is to catch a caller storing the
sentinel); `crates/boyko_physics/src/resources.rs` (`NO_ISLAND = u32::MAX` — shape 2b, the id
needs its bits); `crates/boyko_utils/src/sparse_map/sparse_map.rs` (`sparse:
Vec<Option<usize>>`, 16 bytes per slot).

**After** —

```rust
#[repr(transparent)] #[derive(Clone, Copy, PartialEq, Eq)]
pub struct WorkerId(u8);                          // proves `< MAX_WORKERS == 64` and nothing more (ERG-03); minted by `worker_lane_for` and the idle-word bit scan
#[repr(u8)] #[derive(Clone, Copy, PartialEq, Eq)]
pub enum ThreadRole { Worker(WorkerId), Dispatcher, Unattached }   // STORED: 2 bytes, tag + payload
#[repr(transparent)] #[derive(Clone, Copy, PartialEq, Eq)]
pub struct Lane(u16);
impl ThreadRole {
    /// Derivable for two of the three roles. `Unattached` carries an INDEPENDENT lane (host, a
    /// claimed spare, or unclaimed) — which is why `install` saves the pair instead of deriving it.
    #[inline] pub const fn lane(self) -> Option<Lane> {
        match self { Self::Worker(w) => Some(Lane(w.0 as u16)),
                     Self::Dispatcher => Some(Lane(LANE_DISPATCHER)),
                     Self::Unattached => None }
    }
}
pub struct ThreadLabels { role: ThreadRole, lane: Lane }   // the saved pair, typed — was `Option<(u32, u16)>`
static CURRENT_ROLE: Cell<ThreadRole> = const { Cell::new(ThreadRole::Unattached) };
// consumer: an exhaustive match, no constants, and the lane handed out TYPED, not as a bare u32
match role { ThreadRole::Dispatcher => Lane(worker_count), ThreadRole::Unattached => Lane(0),
             ThreadRole::Worker(w) => Lane(w.0 as u16) }

sparse: Vec<Option<NonZeroU32>>,   // shape 1: stores dense + 1; 4 B per slot; None IS the absent state
```

**What it buys.** "Forgot the dispatcher case" stops being unlikely and becomes impossible: every
consumer is an exhaustive `match`, a fourth role is a compile error at every site that decides,
and a `Worker(WorkerId)` payload cannot be handed to `inner.workers[..]` by accident because the
sentinel arms carry no index. The three-site lane discipline collapses to one write —
`set_role(r)` derives the lane from `r.lane()` where it is derivable — and the one case that is
not derivable is what makes the saved value a `ThreadLabels` pair instead of two integers a
half-restore can split. For the two-state case the absent value stops being a representable
slot, and the `debug_assert_ne!` deletes itself.

**Verified.** EV-71 (this pass): `size_of::<ThreadRole>() == 2`, `align == 1`;
`Cell<ThreadRole>` 2; `Option<ThreadRole>` 2 (the tag has spare niches); `ThreadLabels` 4 and
`Option<ThreadLabels>` 4. `lane_r` (the match over the enum) is 10 instructions against
`lane_c`'s 10 — a `testb` / `cmpl $1` on the tag byte where the sentinel chain has `cmpl $-1` /
`cmpl $-2`; a table-indexing `wake` is 16 = 16 with the tag test replacing the sentinel compare.
Held at codegen-units 1 and 16. The sizes that decide 2a against 2b: `enum { Worker(u32), .. }`
8; `enum { Worker(NonZeroU32), Dispatcher, Unattached }` 8 (a niche buys ONE spare variant);
`#[repr(u8)] enum { Worker(u16), .. }` 4. EV-64: the 2b split is 4 bytes with a 9-instruction
view match. EV-04 / EV-59: shape 1 — the niche loop folds to one symbol with the hand-rolled
sentinel loop; `Option<usize>` 16 bytes vs `Option<NonZeroU32>` 4, and a 2^20-entry sparse
array probed 2^20 times at random went from 27.20 ms to 10.67 ms (2.55×) because the working
set dropped from 16 MiB to 4 MiB — the one finding where the current spelling costs measurable
runtime; `Option<IslandId>` in a guarded loop lost one prologue instruction.

**Exceptions.**
- **A FULL-WIDTH payload in a data-carrying enum is refused as the stored form** (REF-34):
  `enum { Worker(u32), Dispatcher, Unattached }` is 8 bytes, not 4 — a plain `u32` has no niche
  (EV-40, EV-64) — and so is `Option<WorkerId(u32)>`. Ask "does the payload need all its bits?"
  first; only if yes take shape 2b, and then the view is never a field and never the TLS cell.
- A sentinel that is part of a wire, GPU or SIMD-compared format (`VK_QUEUE_FAMILY_IGNORED`,
  `VB_ID_SENTINEL`, `BRICK_OUTSIDE_GRID`, the vector-compared `EMPTY: u64` in `warm_start.rs` /
  `axis_cache.rs`) keeps its integer in the STORED struct; only the API surface changes.
- One niche per type: `Option<Option<X>>` is not 4 bytes. `NonZeroU32::new(x).unwrap()` inside a
  loop is a branch that was not there before — mint at the boundary.
- If the natural encoding is `u32::MAX` and the `+1` bias would spread past two accessors, use
  shape 2 rather than leaking the bias — never a bare constant.
- `#[repr(u8)]` is what PINS the 2-byte layout (the default repr happens to pack it too); gate
  it (ERG-01). The worker-id constants are `pub` and `current_worker_id() -> u32` is a public
  API consumed by `boyko_ecs` and `boyko_physics`: the sweep is cross-crate and its own change.

---

### ERG-05 — `PhantomData` is chosen from the Nomicon table, and `!Send` is pinned explicitly

**Binding:** MUST · **Cost:** ZERO-COST (EV-06, EV-09) · **Guarantee level:** language

**Rule.** Pick the marker by the row you need, not by what is shortest to type:

| Marker | Variance | `Send`/`Sync` | Drop-check | Use for |
|---|---|---|---|---|
| `PhantomData<fn() -> T>` | covariant in `T` | `Send + Sync` always | may dangle | a type TAG (an id that names `T` but stores none) |
| `PhantomData<&'a mut T>` | invariant in `T` | inherited | may dangle | a handle tied to an exclusive borrow |
| `PhantomData<&'a mut &'a ()>` | invariant in `'a` | — | — | a borrow WINDOW (`'scope`) |
| `PhantomData<*const ()>` | — | `!Send + !Sync` | — | pinning a thread-affine guard/view |
| bare `PhantomData<T>` | covariant | **inherited from `T`** | **owns `T`** | ONLY a struct that genuinely owns a `T` it does not store |

**Before** (the wrong row, compiles) — a `Res<'w, R>` state written as `_marker: PhantomData<R>`:
the moment `R` is `!Send`, the state is `!Send`, though it stores only a `ResourceId`.

**After** — the tree already gets every row right: `params/res.rs` `ResState` uses
`PhantomData<fn() -> R>`; `tls.rs` `WorkerDequeDeposit` pins `PhantomData<*const ()>` ("the
deposit describes THIS thread"); `scope.rs` `Scope<'scope>` uses `PhantomData<&'scope mut
&'scope ()>`; `dispatcher_token.rs` uses `PhantomData<&'w mut EcsMaster>`; `scratch/views.rs`
`ScratchBuildView` adds `PhantomData<*mut ()>` beside a `&mut` field "so the negative impl is
explicit and robust against future field changes".

**What it buys.** Variance and auto-trait mistakes never error at the definition site; they error,
or fail to error, at a distant call site. The bare row is the trap: it is what tutorials show.

**Verified.** EV-06: `ResBare<*mut u8>` rejected `E0277`; `ResTag<*mut u8>` accepted; both 4
bytes. EV-09: the ZST marker folds away at the ABI.

**Exceptions.**
- There is no stable static assertion for `!Send` (REF-23); the pin is proven by a trybuild case
  that tries to send it (ERG-10).
- `PhantomData<*const ()>` removes `Sync` too; if `!Send + Sync` is meant, say why.

---

### ERG-06 — A dangerous borrow is handed out as a non-`Copy` token minted by one `pub(crate) unsafe fn`, or as the argument of a caller-supplied closure — never as a handle with a free lifetime

**Binding:** MUST · **Cost:** ZERO-COST (EV-09, EV-10, EV-66) · **Guarantee level:** language
(lifetimes have no representation); measured (codegen)

**Rule.** Two shapes for the same hazard — "the handle outlived or duplicated the thing it
points into":
1. **Token.** A handle wrapping a raw pointer is neither `Copy` nor `Clone` if its value comes
   from uniqueness; its constructor is `pub(crate) unsafe fn` with a `# Safety` naming the
   blessed path; every accessor returning `&mut R` is `fn f(&mut self) -> &mut R` (borrow-tied),
   never `-> &'w mut R`; read views are `&self`-tied. If the token is passed BY VALUE into a
   function whose activation can span a task body, its fields are raw pointers or `NonNull`,
   never references (Miri protects a by-value argument's reference fields for the callee's whole
   activation — `docs/threadpool/KE16-DESIGN-A.md` §1.3 (i)).
2. **Scoped closure.** A borrow that must not escape — the ambient `&PoolInner` read from a raw
   TLS pointer, a `ScratchSolveView` over a buffer that may be refilled — is handed out only as
   `fn with_x<R>(&self, f: impl FnOnce(&X) -> R) -> R`. The elided higher-ranked lifetime means
   the closure cannot name it, so it cannot store, return or send it.

**Before** — the M1 defect this repository shipped and fixed:

```rust
pub fn nonsend_resource_mut<R>(&self) -> Option<&'w mut R>   // two calls ⇒ two live &mut R: UB
```

— and, live in the tree today on the hottest per-task path in the crate,
`crates/boyko_threadpool/src/scope.rs`, `SharedPtr`:

```rust
#[derive(Copy, Clone)] struct SharedPtr { ptr: *const ScopeShared }
unsafe fn as_ref<'a>(&self) -> &'a ScopeShared { unsafe { &*self.ptr } }   // a FREE lifetime from a Copy handle
// Scope::spawn's task wrapper carries the load-bearing invariant in a comment:
let shared = unsafe { shared_ptr.as_ref() };
if let Err(payload) = result { shared.capture_panic(payload); }
// "`shared` MUST NOT be dereferenced after this line" — after `fetch_sub` the joiner may free the box
shared.complete_task();
```

This repository has already had a use-after-free on this allocation (Phase 9.2 Candidate U).
The SAFETY comment is well-formed; the SHAPE is what this rule forbids — checking the spelling
of the comment is not checking the signature.

**After** — `crates/boyko_ecs/src/ecs/core/system/dispatcher_token.rs`, `DispatcherToken<'w>`
(shape 1: `ptr: *mut EcsMaster`, `pub(crate) unsafe fn new`, `nonsend_resource_mut(&mut self)
-> Option<&mut R>`, `world(&self) -> WorldView<'_>`, a `#[cfg(debug_assertions)] owning_thread`);
`crates/boyko_threadpool/src/tls.rs`, `try_with_active_pool<F: FnOnce(&PoolInner) -> R>`
(shape 2), whose SAFETY ends: "the closure cannot capture the borrow because of the
`FnOnce(&PoolInner) -> R` signature; no aliasing escape" — one clause, discharged by the
signature. And for `SharedPtr`, shape 1 with the last access consuming the handle:

```rust
struct SharedPtr { ptr: NonNull<ScopeShared> }              // not Copy, not Clone; no reference ever leaves
impl SharedPtr {
    fn capture_panic(&self, p: Box<dyn Any + Send>) { /* SAFETY: the mint's contract; `&self`-tied reborrow */ }
    /// The LAST access, by construction: the handle is consumed, so no path can reach the
    /// pointer after the decrement that lets the owner free the box.
    fn complete(self) { unsafe { self.ptr.as_ref() }.complete_task() }
}
// the wrapper:  if let Err(p) = result { shared_ptr.capture_panic(p); }  shared_ptr.complete();
```

**What it buys.** N `unsafe` sites each re-proving "I am on the dispatcher" collapse into ONE
mint; a worker cannot obtain the token (C1); borrowck refuses the double projection (M1); a
handle with a free lifetime — the thing the M1 Before is — becomes unwritable in shape 2; and
"must not be used after this line" becomes a use-after-move error instead of a sentence.

**Verified.** EV-09: a ZST token parameter folds away. EV-10: the `ThreadId` field is absent from
the release type. EV-66: a scoped accessor `with_pool(|p| …)` and the free-lifetime `-> &'a
Pool` form folded to ONE symbol (`sc_scoped = sc_direct`) at codegen-units 1 and 16; the
closure that tries to return its borrow is a lifetime error at compile time. EV-72 (this pass):
the task body over the `Copy` `SharedPtr` (`as_ref<'a>`, then `capture_panic` / `complete_task`
through the reference) and over the non-`Copy` handle (`complete(self)`) are 22 instructions
each with 0 normalised-diff lines at codegen-units 1 and 16 — a move of one pointer is the same
machine code — and a deref after `complete()` is `E0382: borrow of moved value`. M1 and C1 are
pinned by `crates/boyko_ecs/tests/compile_fail_dispatcher_token.rs` (ERG-10).

**Exceptions.**
- Borrowck exclusion is single-thread exclusion; the cross-thread half needs the `!Send` pin
  (ERG-05) or a non-`'static` token.
- Shape 2 is one instantiation per closure body: keep the closure thin and the work in a concrete
  inner `fn` (ERG-15). Two nested scoped APIs drift right; a body that needs `?` or `continue`
  uses `ControlFlow` (ERG-38).
- Shape 2 is NOT branded / generative lifetimes (REF-05): it introduces no `'id` parameter.

---

### ERG-07 — A worker never receives a whole-buffer `&mut [T]`: range-partitioned work gets disjoint chunk slices; a colour-partitioned buffer gets a build view and a solve view with no slice surface

**Binding:** MUST · **Cost:** ZERO-COST, structural (method absence) · **Guarantee level:**
language

**Rule.** Two shapes, decided by how ownership is partitioned:
1. **Range-partitioned** (each worker owns a contiguous `[start, end)`): mint one disjoint
   `&mut [T]` per worker on the build side with `split_at_mut` / `chunks_exact_mut` and hand each
   worker its slice — the `par_chunk` shape (ERG-24).
2. **Colour-partitioned** (a worker owns a scattered set of indices no slice can express): the
   single-threaded build view (`!Send`, ERG-05) is the ONLY type with `as_mut_slice` / `Deref`.
   The solve view is `Copy + Send + Sync` and exposes `unsafe fn row_ptr(&self, i) -> *mut T`,
   `len()` and, on the dense twin, a liveness predicate — and NO method returning a slice.
   Adding one "just for a test" is what a trybuild case rejects (ERG-10).

**Before** — the O11-SP4 colored-solve data race: a whole-buffer `&mut [T]` handed to parallel
workers with the instruction "only touch your own indices". A discipline, not a type.

**After** — `crates/boyko_ecs/src/ecs/core/iters/query/par_chunk.rs` (shape 1: each spawned task
receives one `&'c mut [T]` for its `[start, end)`, "CD3 disjointness … satisfied structurally");
`crates/boyko_ecs/src/ecs/core/component/scratch/views.rs` and `dense/views.rs` (shape 2:
`ScratchBuildView` / `ScratchSolveView`, `DenseBuildView` / `DenseSolveView`).

**What it buys.** A worker cannot write the type of the thing it must not do. `row_ptr` is one
`add`; where the partition IS contiguous the slice form hands LLVM two provably disjoint
regions.

**Verified.** Method absence is checked by the compiler. EV-60 / EV-70: on rustc 1.97.1 the
chunk-slice form is at worst equal to raw per-row pointers (both vectorise) — the reason to
prefer shape 1 where it applies is review budget, not cycles.

**Exceptions.**
- The split does not prove index DISJOINTNESS; the colouring does. `row_ptr`'s `# Safety` says so.
- The solve view carries only a lifetime tie; the API that hands it out — a closure-scoped
  `with_solve_view` (ERG-06 shape 2) or the scope's `'scope` — bounds it.

---

### ERG-08 — A two-phase protocol or a mutually exclusive builder mode is a ZST typestate marker — for SMALL payloads only

**Binding:** SHOULD (a few words of payload); MUST-NOT consume `self` per transition over a large
inline payload across a non-inlined boundary · **Cost:** ZERO-COST when the payload is small
(EV-07, EV-59); COSTS a 4 KiB `memcpy` per transition otherwise (EV-08)

**Rule.** `struct Map<Phase> { entries: Vec<_>, _phase: PhantomData<fn() -> Phase> }`; the
methods legal in one phase live on `impl Map<Inserting>`, the transition is `fn finalize(self)
-> Map<Sealed>`, and the other phase's methods do not exist on the first. Gate
`size_of::<Map<Phase>>() == size_of::<Payload>()` (ERG-01). Stop at two or three states; never
typestate an optional field.

**Before** — `crates/boyko_ecs/src/ecs/core/serialize/mod.rs`, `LoadEntityMap`: a debug-only
`sealed: bool`, so in release `get` before `finalize` is an UNSORTED binary search — a wrong
`Entity` or a wrong `None`, in the loader, with no panic. Both `#[cfg(debug_assertions)]` blocks
carry a comment explaining why the assert must itself be gated.

```rust
pub struct LoadEntityMap { entries: Vec<(u64, Entity)>, #[cfg(debug_assertions)] sealed: bool }
pub fn insert(&mut self, ..) { #[cfg(debug_assertions)] debug_assert!(!self.sealed, ..); .. }
pub fn get(&self, ..) -> Option<Entity> { #[cfg(debug_assertions)] debug_assert!(self.sealed, ..); .. }
```

**After** —

```rust
pub struct Inserting; pub struct Sealed;
pub struct LoadEntityMap<Phase = Inserting> { entries: Vec<(u64, Entity)>, _phase: PhantomData<fn() -> Phase> }
impl LoadEntityMap<Inserting> {
    pub fn insert(&mut self, saved: usize, fresh: Entity) { self.entries.push((saved as u64, fresh)); }
    pub fn finalize(mut self) -> LoadEntityMap<Sealed> {
        self.entries.sort_unstable_by_key(|&(k, _)| k);
        LoadEntityMap { entries: self.entries, _phase: PhantomData }
    }
}
impl LoadEntityMap<Sealed> {
    pub fn get(&self, saved: usize) -> Option<Entity> { /* binary search */ }
}
```

(`crates/boyko_ecs/src/ecs/core/clone/cloner.rs`, `EntityClonerBuilder<Mode>`, is the
builder-mode instance already in the tree.)

**What it buys.** The check moves from debug-only runtime to always-compile-time; both cfg blocks
and the flag disappear; by-value `finalize(self)` makes double-finalize impossible too.

**Verified.** EV-59: `get` over the flag form and over the sealed typestate emitted
label-normalised IDENTICAL 41-instruction bodies, and the typestate is SMALLER — 24 bytes against
32 (the `bool` plus seven bytes of padding are gone). EV-07: a consuming chain over a small
payload folds to the literal. EV-08: over a 4096-byte inline payload across `#[inline(never)]`
transitions, a 4104-byte `memcpy` per step, ~190× slower.

**Exceptions.**
- Principle 3 wins: N states × M methods is N·M instantiations; state-independent methods go on
  `impl<Phase>`.
- A state chosen at RUNTIME is not a typestate; `Box<dyn State>` for it is REF-02.
- A large payload transitions by `&mut self` plus a separate proof token (ERG-06), or is boxed.

---

### ERG-09 — A kernel trait whose implementors must uphold an `unsafe` invariant is sealed, and the site says which seal

**Binding:** MUST · **Cost:** COMPILE-TIME ONLY (EV-11) · **Guarantee level:** language

**Rule.** `mod sealed { pub trait Sealed {} }` in a PRIVATE module for a hard seal. If a derive
macro must emit the impl downstream, the module is `#[doc(hidden)] pub mod sealed` and the doc
says "a discoverability boundary, not an enforcement". A trait that carries an `unsafe`
contract, or whose associated consts the kernel trusts, is not left open.

**Before** — `crates/boyko_utils/src/bit_mask/bit_set.rs`, `BitInteger`: public, unsealed. A
downstream `impl BitInteger for Foo { const BITS: usize = 1000; … }` makes every
`debug_assert!(index < T::BITS)` in `BitSet` vacuous and shifts past the width.

**After** — `crates/boyko_ecs/src/ecs/core/app/plugins.rs`: `mod sealed { pub trait
Sealed<Marker> {} }`, `pub trait Plugins<Marker>: sealed::Sealed<Marker>`. For `BitInteger`:
`pub trait BitInteger: sealed::BitIntegerSealed + Copy + …` with one `impl
sealed::BitIntegerSealed for u8 {}` per blessed type.

**What it buys.** The safety argument of `BitSet` rests on `BITS` telling the truth; the seal
makes that a compiler fact at no runtime trace.

**Verified.** EV-11: sealed-with-marker and plain trait folded to one symbol.

**Exceptions.**
- A `pub use` of the sealed module silently unseals. With a `Marker` parameter, the supertrait
  carries the SAME marker or the impls overlap again.
- `crates/boyko_ecs/src/ecs/core/bundle/bundle.rs`'s first paragraph above `mod sealed`
  describes a hard seal for a module that is `pub mod sealed` — doc-rot to delete on next edit.

---

### ERG-10 — Every "no longer compiles" property that a later impl, derive or method could silently undo ships with a trybuild compile-fail case in the same commit

**Binding:** MUST (scoped as below) · **Cost:** TEST-ONLY · **Guarantee level:** language

**Rule.** ERG-05 (`!Send` pin), ERG-06 (double projection, escaping closure), ERG-07 (no slice
surface on the solve view), ERG-08 (wrong-phase call), ERG-09 (downstream impl) are claims about
what does NOT compile; nothing in a normal test observes that. Add a `.rs` under
`tests/compile_fail_<topic>/` and a `.stderr` baseline, and read the diff when re-blessing. The
MUST is scoped to properties a later `impl`, derive or method can silently UNDO: a `!Send` pin
(one field change), `IdleWord` gaining a `BitAnd` (which makes the folded call compile again —
EV-73's second refusal), the absence of a slice surface on a solve view (one convenience
method), a wrong-phase typestate call (a method moved to `impl<Phase>`), a downstream impl of a
sealed trait (a `pub use`). A plain argument transposition between two distinct nominal newtypes
(`claim_one_idle(inner, exclude, observed, ..)` → `E0308`) owes NO fixture: nominal typing cannot
evaporate without a `Deref` (ERG-17) or a `From`, so the fixture pins nothing that can change
while costing a re-bless on every toolchain move — this corpus went red 23 fixtures at a time on
one rustc bump (`docs/OPEN-QUESTIONS.md`, 2026-08-11; REF-42).

**Before** — a type-level guarantee documented in prose: the day someone adds a convenience
method or derives `Copy`, every existing test still passes.

**After** — `crates/boyko_ecs/tests/compile_fail_dispatcher_token.rs` (three cases); twelve
further `compile_fail_*` suites under `crates/boyko_ecs/tests/` and one under
`crates/boyko_log/tests/`.

**What it buys.** The guarantee is exactly as strong as the test that goes red when it evaporates.

**Verified.** Test-only; the suites run in the ordinary `cargo test --workspace` gate.

**Exceptions / traps.**
- **An empty glob exits 0.** `t.compile_fail("tests/does_not_exist/*.rs")` passes silently; this
  repository has been burned by it (`tests/trybuild_corpus_compiler_witness.rs` exists for that
  reason). A new suite must be counted by that witness.
- A case that fails for the WRONG reason still passes; the `.stderr` baseline pins the reason.
  `TRYBUILD=overwrite` without reading the diff blesses a regression in.
