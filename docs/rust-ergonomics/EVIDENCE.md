# RUST-ERGONOMICS — evidence ledger

Every cost verdict in [RUST-ERGONOMICS.md](../RUST-ERGONOMICS.md) and its section files cites a row
here by `EV-nn`. A rule never carries a number that is not in this ledger, and this ledger never
carries a number that was not produced on this checkout. A row that says a claim did NOT hold up is
kept on purpose: a refuted claim is the most useful entry in the file.

## Method

**Toolchain.** `rustc 1.97.1 (8bab26f4f 2026-07-14)`, `stable-x86_64-pc-windows-gnu` — the
engine's toolchain. ⚠️ The box's *default* rustup toolchain is `stable-x86_64-pc-windows-msvc`
1.92.0; a bare `rustc`/`cargo` without a toolchain override selects it and produces different
codegen. Every row below was produced with the gnu 1.97.1 toolchain selected explicitly
(`rustup run stable-x86_64-pc-windows-gnu …` or the verification crate's toolchain override).

**Profile.** Edition 2024, `opt-level = 3`, **no LTO**, default x86-64 baseline (SSE2). Rows
EV-01..EV-58 were produced at `codegen-units = 1`. ⚠️ **That is NOT the shipped release
profile** (corrected 2026-09-02 by the runtime-cost guard): the workspace has no
`[profile.release]` section — only `[profile.bench]` pins `codegen-units = 1` — so `cargo build
--release` uses cargo's default of 16 units. LLVM's identical-code folding runs only WITHIN a
unit, so at 16 units two byte-identical functions may produce no alias line purely by
partitioning: **the absence of an alias is not evidence of difference at the shipped profile.**
Every identity in rows EV-59..EV-70 was re-checked at 16 units (the aliases that matter appeared,
and the non-aliased pairs diffed to 0–2 lines); no verdict flipped. The SSE2 baseline IS the
shipped ISA: no `.cargo/config.toml`, `Cargo.toml` or CI file sets `target-cpu` /
`target-feature`, and `boyko_physics::solver::simd`'s AVX2 arms are `cfg(target_feature =
"avx2")`-gated with a scalar fallback. `CLAUDE.md` names AVX2 as the baseline; the build
configuration does not enable it — an owner item (index, OPEN 2). Absolute ns/element figures
would shift under `+avx2`; the identity results would not.

**Where.** A scratch Cargo workspace under the system temp directory (rows EV-01..EV-41), a
single-file `rustc --emit=asm` probe in the session scratchpad (rows EV-42..EV-44), a second
scratch crate in the session scratchpad with a counting `#[global_allocator]` (rows EV-45..EV-55,
same profile and toolchain, `rust-toolchain.toml` pinning the gnu channel), a third
(`lintprobe`, same pinning and profile) for rows EV-56..EV-57, and a fourth (`eeprobe`, same
pinning and profile, `cargo build --release` with `--emit=asm`) for row EV-58. Nothing under
`crates/` was built, modified or linked into any probe.

**Four methods**, named per row:

- **L (layout)** — `const _: () = assert!(size_of / align_of / offset_of …)` compiled in the
  release profile. Compiling IS the proof; a false claim is an `E0080` build failure.
- **A (assembly)** — `--emit=asm`, then reading the `.s`. At `codegen-units = 1` LLVM performs
  identical-code folding and emits an explicit alias line `sym_a = sym_b` when two functions have
  byte-identical bodies. **An ICF alias is the strongest identity evidence available** — stronger
  than a diff. Where functions did not fold: a label-normalised diff, an opcode histogram, or a
  per-element steady-state instruction count.
- **T (timing)** — hand-rolled harness, best-of-7 × N reps after warm-up, `black_box` on operands
  and result, ns per element; every figure reproduced across ≥ 2 independent process runs.
- **B (behaviour)** — two-crate workspaces for cross-crate properties, deliberate compile-fail
  probes for auto-trait and lint claims, one runtime probe printing a panic `Location`.
- **C (allocation count)** — a `GlobalAlloc` wrapper over `System` counting `alloc` / `dealloc` /
  `realloc` with relaxed atomics; deltas read around the call under test, the result dropped
  outside the counted region. This is the instrument EV-29 said was missing.

**Reading rule.** Instruction count is NOT a cost proxy in either direction: EV-02's faster loop
has MORE instructions (unrolling), EV-22's slower one has FEWER. Any verdict here that rests on a
count alone says so. **Second reading rule (EV-49):** a wall-clock delta over BYTE-IDENTICAL
bodies is code placement, not the technique — the ledger accepts a timing verdict only beside an
asm identity, a body diff, or an allocation count.

## Rows

**EV-01 — `#[repr(transparent)]` newtype over `u32` vs raw `u32`.** (L, A) 38 layout gates green;
summing loop over `&[Id]` vs `&[u32]` produced the ICF alias `a_raw = a_newtype`. Byte-identical.
Layout half is a Reference guarantee; codegen half is now measured. → ZERO-COST.

**EV-02 — private-field newtype + `get_unchecked` vs bounds-checked `u16` index.** (A, T) 4096
elements. Checked: 7 instr/element (`movzwl, cmpq $4096, jae, xorq, addq, cmpq, jne`); proven:
2 instr/element because removing the check let LLVM unroll 8×. Timing 0.527–0.547 ns/elem checked
vs 0.357–0.359 proven = **1.5× faster**, two runs. The unchecked version has MORE total
instructions (46 vs 30) — unrolling, not cost. → ZERO-COST, removes work. **SUPERSEDED as a
speed claim (2026-09-02, EV-60): on a 16 KB `f32` table and a 256 KB component column the time
delta does not reproduce; the codegen change does. No longer licenses `get_unchecked` — ERG-03.**

**EV-03 — `const _` layout gates.** (L) 38 gates compiled; they emit no code. The gate
`size_of::<DspBuf<1>>() == 1 + size_of::<usize>()` FAILED with `E0080` — real answer 16, the
`[u8; 1]` is tail-padded to `usize` alignment. The gate caught a wrong belief in the guide's own
sketch within seconds. → COMPILE-TIME ONLY, and load-bearing.

**EV-04 — `Option<Newtype(NonZeroU32)>` niche.** (L, A) `size_of::<Option<SlotIndex>>() == 4`,
align 4; the niche survives two nested `repr(transparent)` layers. A loop over
`&[Option<NonZeroU32>]` and a hand-rolled `0`-sentinel loop over `&[u32]` folded to one symbol
(`v_sentinel = v_niche`). → ZERO-COST; the `Option` IS the sentinel convention.

**EV-05 — `Option<NonNull<T>>` is one word.** (L) `== size_of::<usize>() == 8`, also for a
4096-byte pointee. → ZERO-COST (std `Option` representation guarantee).

**EV-06 — `PhantomData` table: `fn() -> R` vs bare `R`.** (L, B) Both ZST (size 0, align 1).
`ResTag<*mut u8>` with `PhantomData<fn() -> R>` passes `assert_send`; `ResBare<*mut u8>` with
bare `PhantomData<R>` FAILS `E0277` ("`Send` is not implemented for `*mut u8` … required because
it appears within the type `PhantomData<*mut u8>`"). Both 4 bytes. → the table row distinction is
real and load-bearing; bare `PhantomData<T>` as a tag COSTS a wrong auto-trait relation.

**EV-07 — typestate, SMALL payload, consuming transitions.** (L, A)
`size_of::<Builder<OptIn>>() == size_of::<Inner>()`; the chain
`SmallBuilder::new().step1().step2().build()` folded to the literal (`z_small_chain = z_direct`).
→ ZERO-COST.

**EV-08 — typestate, LARGE payload (4096-byte inline array), 4-state chain.** (A, T) Transitions
`#[inline]`: whole chain compiles to `movl $6, %eax; retq`, ICF-folds with the `&mut self` form,
1.15–1.18 ns. Transitions `#[inline(never)]`: each transition emits `callq memcpy` of 4104 bytes,
caller frame `.seh_stackalloc 8248` plus a `___chkstk_ms` probe, 223–234 ns — **~190× slower**.
→ ZERO-COST only when the chain inlines; COSTS a 4 KiB memcpy per transition when it does not.

**EV-09 — zero-sized capability token as a parameter.** (L, A) `size_of == 0` for both the
`PhantomData<*const ()>` and `_private: ()` forms; the token-carrying and token-free functions
folded (`w_with_token = w_no_token`). → ZERO-COST at the ABI.

**EV-10 — `#[cfg(debug_assertions)]` field inside a type.** (L) Release-profile gate
`size_of::<TokenDbg>() == size_of::<usize>() == 8`: the `ThreadId` tripwire is absent from the
type, not merely unused. Compilation-conditional ⇒ guarantee. → ZERO-COST in release.

**EV-11 — sealed trait + `Marker` type parameter vs plain trait.** (A) `sl_sealed = sl_plain`.
→ COMPILE-TIME ONLY.

**EV-12 — `impl Trait` in argument position vs named generic.** (A) `i_call_generic = i_call_apit`.
→ ZERO-COST (documented sugar); the turbofish/semver hazard is real but not a runtime cost.

**EV-13 — return-position `impl Iterator` vs `Box<dyn Iterator>`.** (A, T) Boxed: 69
instructions including `callq ___rust_no_alloc_shim_is_unstable_v2` (a real allocation). RPIT:
44, no allocator call. Timing 0.083–0.086 ns/elem RPIT vs 0.154 boxed = **1.8×**. → RPIT
ZERO-COST; `Box<dyn Iterator>` COSTS.

**EV-14 — dispatch: `#[repr(u8)]` enum tag vs `&dyn` vs fn pointer, TWO loop shapes.** (A, T)
Asm: enum tag = zero `call` instructions, loop-invariant `match` hoisted, body vectorised
(`paddq/pxor/movdqu`); `&dyn` = `callq *%r15`; fn ptr = `callq *%rbx`. **Latency-bound loop**
(serial dependency chain): dyn 1.96, tag 1.96–2.12, fnptr 2.01–2.06 ns — NO measurable difference.
**Throughput-bound loop**: tag 0.233 ns/elem = monomorphic 0.233–0.236; dyn 1.163–1.172; fnptr
1.175–1.253 — **5×**. → enum tag ZERO-COST vs monomorphic; `dyn`/fn-ptr COST 5× on throughput
loops and are invisible on latency-bound ones. State the loop shape with every dispatch claim.

**EV-15 — `Option<fn>` size and fn-pointer call cost.** (L, T) `size_of::<Option<unsafe fn(*const
u8, *mut u8)>>() == 8`, `Option<extern "C" fn()> == 8`. Call cost from EV-14: fn ptr 1.175–1.253
vs dyn 1.163–1.172 ns/elem — NOT cheaper to call. → size ZERO-COST; the honest benefit is no
`Box` allocation and a thin pointer, not a cheaper call.

**EV-16 — `Deref<Target = [T]>` on a buffer newtype.** (A) `m_direct = m_deref`. → ZERO-COST.

**EV-17 — `Deref<Target = u64>` on a domain newtype.** (B) Without `Deref`, downstream
`d.count_ones()` is `E0599`. With a two-line `Deref`, `d.count_ones()`, `d.leading_zeros()`,
`d.wrapping_mul(3)` and `*d` all compile from the downstream crate. → COSTS the entire
encapsulation; runtime free.

**EV-18 — outer-generic / inner-concrete split.** (A, T-build) Instruction lines in the crate's
own `.s`: n=1 generic 67 / split 68; n=8 531 / 189 (2.8×); n=32 2132 / 614 (3.5×). Cold build at
n=32: 742 ms generic vs 554 ms split (+25 %). A first attempt over 9 `AsRef<Path>` types showed
0.24 % `.text` delta — ICF deduplicated identical bodies and std dominated the executable; the real
number appears only when bodies genuinely differ. → COMPILE-TIME + I-cache cost of the generic
form, linear in instantiations.

**EV-19 — const-generic `bool` flags vs runtime `bool`.** (A, T-build) Calling the runtime-bool
function with LITERAL arguments folded to the same symbol as the const-generic version
(`k_rt_tf = k_const_tf`) — const generics bought nothing over literals. Against an opaque bool:
188 instr, 0.084 vs 0.088 ns/elem — no measurable win. 5 flags = 32 instantiations = 1182
instructions vs 395 for one runtime function (3×), build 819 ms vs 593 ms (+38 %). → COSTS
monomorphisation for a branch that was already folded or well predicted.

**EV-20 — iterator adaptors vs index loops, four claims.** (A, T) (a) `iter().fold` vs while-index:
identical modulo labels, 0.725–0.755 vs 0.726–0.742 ns/elem. (b) `for &x in xs`: different loop
shape (peel-then-unroll vs unroll-then-remainder), 41 vs 37 instr, same 0.724 ns/elem.
(c) `filter().map()` vs `filter_map()`: 71 vs 52 instr, 0.147–0.158 vs 0.159 — no difference.
(d) `chunks(8)` vs `chunks_exact(8)`: 45 vs 18 instr, 0.744 vs 0.741 — no difference at N
divisible by 8. None vectorised (f32 add is not associative). → per-site; identity is usual, not
guaranteed.

**EV-21 — `iter().chain()` vs two sequential loops.** (A, T) `chain` is SMALLER (68 vs 87
instructions) and SLOWER: 0.063–0.072 vs 0.035–0.042 ns/elem — **~1.7×**. The one case where
reading the asm alone gives the wrong verdict. → COSTS. **QUALIFIED (EV-60): 3–8 % at column
scale; the direction reproduces in every run, the 1.7× magnitude is L1-resident scale.**

**EV-22 — `split_at_mut` vs hand-rolled raw-pointer disjointness.** (A, T) `split_at_mut`: 53
instr, 2 `addps` + 1 `addss` + 6 `movups` (vectorised). Raw pointers off one base: 77 instr,
2 `addps` + 5 `addss` + 10 `movss` — scalar fallback, LLVM cannot prove the two derefs disjoint.
0.140 vs 0.175 ns/elem — the unsafe version is **25 % slower**. → the safe API wins.
**SUPERSEDED (2026-09-02, EV-60, EV-70): on rustc 1.97.1 at the shipped profile the raw form
vectorises too (`addps` in both) and times identically; ERG-24 now rests on review budget, not
speed.**

**EV-23 — `Index<Id>` vs `get_unchecked` on the hot path.** (A, T) `Index` keeps `cmpq/jbe` per
element plus a cold `panic_bounds_check` tail, ~7 instr/elem; unchecked unrolls 8× to ~2.3.
0.514–0.571 vs 0.359–0.421 ns/elem. → `Index` COSTS on a proven-in-range path. **QUALIFIED
(EV-60): the asm delta is real, the time delta did not reproduce on the shapes this engine
iterates; REF-15 is a preference, not an `unsafe` licence.**

**EV-24 — `#[track_caller]`.** (A, B) Untracked call site: one instruction (`jmp t_plain`).
Tracked: two (`leaq .Lanon…(%rip), %rdx; jmp t_tracked`) plus a `Location` static — an extra
argument at EVERY call site. Behaviour: direct call printed the caller's location; the SAME fn
through a fn-pointer coercion printed the DEFINITION site (`dep\src\lib.rs`). → COSTS, and silently
stops working through fn-pointer tables.

**EV-25 — `#[must_use]` lint holes.** (B) Of 10 statements, exactly 2 warned: bare `f();` and
bare `make_guard();`. SILENT: `(f(),);`, `Some(f());`, `if true { f() } else {0};`,
`match true { _ => f() };`, `let _ = f();`, `_ = f();`, and `let _ = make_guard();` — which drops
the guard immediately. Zero code cost. → a nudge, not enforcement.

**EV-26 — `#[non_exhaustive]` from a downstream crate.** (B) Struct literal `E0639`; struct
pattern without `..` `E0638`; exhaustive match without `_` `E0004`. CORRECTION: `Fmt::Rg8 as u32`
with `#[non_exhaustive]` on the ENUM compiled fine; the cast is blocked only with the attribute on
a VARIANT (`E0606`). → downstream exhaustiveness is lost; the `as`-cast claim was overstated.

**EV-27 — `#[inline]` on cross-crate methods, no LTO, three body sizes.** (A, B) Tiny and ~20-line
bodies: inlined and ICF-folded WITHOUT the attribute (`u_noinline = u_inline`,
`u_big_noinline = u_big_inline`). 120-statement body: with `#[inline]` 965 instr inlined; without,
12 instr and a real `callq` into the other crate. → `#[inline]` is redundant below rustc 1.97.1's
`cross_crate_inlinable` threshold and load-bearing above it. Compiler behaviour, not a guarantee.

**EV-28 — `..Cfg::DEFAULT` vs `..Default::default()` on a 6-field POD.** (A)
`n_fru_default = n_fru_const`. → identical for POD.

**EV-29 — `..Default::default()` where `Default` ALLOCATES.** (A, T) FRU form shows 2
`__rust_no_alloc_shim` sites vs 1, but BOTH show one `__rust_alloc` and one `__rust_dealloc` —
LLVM eliminated the dead `String::from`. Timing 184.8 vs 183.6 ns, then 61.6 vs 61.9 on rerun:
allocator-noise dominated, cannot resolve one malloc. → **could-not-check** on this row; needs a
counting global allocator. **RESOLVED by EV-45**, which counted it: the overridden base field IS
allocated and freed.

**EV-30 — operator overloading on a 3-field `Copy` vector vs per-field arithmetic.** (A)
Identical opcode histogram across all 36 distinct opcodes (4 `addps`, 4 `mulps`, 27 `movss`, …);
did not ICF-fold only because of scheduling/register differences. Both vectorised. → ZERO-COST.

**EV-31 — `IntoIterator for &T` delegating to `iter()`.** (A) `it_forloop = it_explicit`.
→ ZERO-COST.

**EV-32 — `#[cold] #[inline(never)]` panic helper.** (A) Hot function 13 instructions with the
helper vs 21 with inline `panic!`s; format-string setup and both panic sequences moved out.
→ I-cache win; the code MOVEMENT is structural, only the attributes are hints.

**EV-33 — `matches!` / let-else / labelled blocks.** (A) `r_matches = r_match`. → ZERO-COST.

**EV-34 — `#[derive(Debug)]` on a 16-field struct + 30-variant enum vs hand-written 3-field
`Debug`.** (A, T-build) Derived: 461 instr, 35 string-data directives, 640 ms. Hand-written: 154,
5, 447 ms. → 3× code, 7× string data, +43 % build; runtime zero on non-formatting paths.

**EV-35 — the published GAT lending-iterator sketch.** (B) `type Item<'a> where Self: 'a = &'a [T];`
warns `deprecated_where_clause_location` (issue #89122) — under `-D warnings` it FAILS. Corrected
`type Item<'a> = &'a [T] where Self: 'a;` compiles clean and runs. → syntax correction; GATs are
compile-time only.

**EV-36 — `DspBuf<const N>` size.** (L) The `N = 64` gate passes; `N = 1` is 16 not 9, `N = 63` is
72 not 71 — tail padding to `usize` alignment. → the general form of that gate is false.

**EV-37 — `&raw const` vs `&x as *const _`.** (A) `p_ref_cast = p_raw_op`. → ZERO-COST; the
aliasing-model benefit cannot be shown by asm and was NOT Miri-checked here.

**EV-38 — by-value `self` vs `&self` on a `Copy` pointer-sized cell.** (A) `q_byval = q_byref`.
→ ZERO-COST, codegen identity only; the guide draws no aliasing conclusion from this row
(ERG-23's receiver clause is a MAY).

**EV-39 — `impl Into<String>` probe.** (A) `y_into::<&str>(s).len()` compiled to 15 instructions
with the malloc/free pair eliminated because only `.len()` was live. → **probe too weak**; the
refusal stands on mechanism (an `Into<String>` call site that keeps the `String` allocates by
definition) and on EV-18 for the monomorphisation half.

**EV-40 — layout facts behind the dispatch arguments.** (L) `Box<dyn Fn(u32) -> u32>` = 16 =
`&dyn Debug`; `Option<Box<dyn Fn>>` = 16 (niche); `#[repr(u8)] SystemKind` = 1 and
`Option<SystemKind>` = 1; `Row<C>` with `PhantomData<fn() -> C>` = 4 but `Option<Row<C>>` = **8**
(a plain `u32` has no niche). → all gates green.

**EV-41 — `#[repr(transparent)]` bitflag newtype, `const fn` accessors.** (L) size/align equal to
`u32`; covered by EV-01's codegen identity. → ZERO-COST.

**EV-42 — bare `bool` parameter vs two-variant `#[repr(u32)]` enum (this pass).** (A) At an
inlined call site with a literal: `fc_enum = fc_bool`. Non-inlined callee building a
`{ stype, flags }` record: bool form `movzbl %cl, %edx` vs enum form `movl %ecx, %edx` — one
instruction each. → ZERO-COST.

**EV-43 — `a |= b` (`BitOrAssign`) vs `a = a | b` on a `repr(transparent)` flag newtype (this
pass).** (A) `fl_or_assign = fl_or`. → ZERO-COST.

**EV-44 — `SparseMap::get`: manual guard-then-index vs `get(..).copied().flatten().and_then(..)`
(this pass).** (A) BOTH compile to exactly three compares (`sparse.len`, the `Some` tag,
`dense.len`) — LLVM had already deleted the manual form's redundant `self.sparse[index]` check.
The chain form carries one extra `mov`. The map's claim "strictly fewer branches" did NOT hold.
Over `Vec<Option<NonZeroU32>>`: the same three compares with a 4-byte-stride load
(`movl (%rax,%rdx,4)`) where the `Option<usize>` form loads at a 16-byte stride (`shlq $4`).
→ chain form is a clarity change at zero cost, not a branch win; the niche form is the real win.

**EV-45 — `..Default::default()` where `Default` allocates.** (C) A `Default` that builds a
`String` and a `Vec::with_capacity(8)`; `Cfg { name, ..Default::default() }` costs **2 alloc +
1 dealloc** per construction against 1 + 0 for the hand-written literal; overriding the `Vec`
instead of the `String` gives the same 2 + 1 vs 1 + 0. Reproduced across two calls and two process
runs. → COSTS one malloc + one free per construction. Resolves EV-29.

**EV-46 — `collect()` / `extend()` with an exact `size_hint` vs the default `(0, None)`.** (C, T)
`collect` from an iterator with the default hint: n = 64 → 1 alloc + **4 realloc**; n = 4096 → 1
alloc + **10 realloc**. With an exact `size_hint` + `ExactSizeIterator`: 1 + 0 at both sizes.
Timing at 4096: 0.66–0.72 vs 0.46 ns/elem (**~1.5×**). `extend` into a `Vec::with_capacity(n)`:
1 + 0 regardless of the hint. → an exact hint is load-bearing for `collect`; irrelevant once the
caller reserved.

**EV-47 — `debug_assert!` and release `assert!` in the release profile.** (A) `da_with` (a
`debug_assert!` before `xs[i]`) vs `da_without`: 11 instructions each, 0 normalised-diff lines —
the one compare that remains belongs to the indexing. `ra_debug` (a `debug_assert!` on a
shift/or) ICF-folded to the assert-free `ra_none` (`ra_none = ra_debug`). `ra_release` (the
`slot_to_u64` shape, release `assert!`): `shrq $32; jne` on the hot path plus a cold `panic_fmt`
block. → `debug_assert!` ZERO-COST in release (compilation-conditional ⇒ guarantee); a release
`assert!` costs two hot instructions and a cold block.

**EV-48 — `expect("invariant: …")` vs `unwrap()`.** (A) Hot path identical (`cmpl $1; jne; movl;
ret`). Cold block: `expect` loads the message pointer and length and calls `expect_failed` (11
instructions total) vs `unwrap_failed` (9). → ZERO-COST on the hot path; the message costs two
cold instructions and a rodata string.

**EV-49 — associated-`const` predicate `if D::HAS_DENSE { … }`.** (A, T) `ac_table`
(`HAS_DENSE = false`) vs a hand-written loop with no arm: 39 instructions each, **0
normalised-diff lines**, both vectorised (`paddd`, 11 vector ops). `ac_dense` (`= true`) keeps
the mask arm: scalar, 0.55 ns/elem vs 0.030. **Caveat recorded:** in the first build the two
identical bodies timed 0.058 vs 0.030 ns/elem, reproduced across two process runs; after an
unrelated edit to `main` moved the code, both timed 0.0298–0.0300 in either call order. → arm
deletion ZERO-COST; and a wall-clock delta over byte-identical asm is code placement.

**EV-50 — `const fn` accessor taking `self` vs `fn(&self)` on a `Copy` value type.** (A, L)
`cf_plain = cf_const`; the gate `const _: () = assert!(Slot::new(7, 1).index_c() == 7)`
compiled. → ZERO-COST; `const` restricts the body, it does not change codegen.

**EV-51 — `zip` vs an index loop over two slices.** (A, T) Index form: 67 instructions, a
`leaq -1 / cmpq / jbe` pre-check and a `panic_bounds_check` block for `b[i]`. `zip`: 61, no panic
block, **0 diff lines** against the hand-hoisted index form (`let n = a.len().min(b.len())`).
All three vectorised (28 vector ops); 0.145–0.147 ns/elem each. → ZERO-COST; `zip` IS the hoisted
form. Semantics differ on unequal lengths (`zip` truncates, indexing panics).

**EV-52 — config fields read through `&Cfg` inside a loop vs hoisted into locals.** (A, T)
Through `&Cfg` and `&mut [f32]`: **0 diff lines** — LLVM hoists the loads itself. Through
`*const Cfg` and `*mut f32`: the un-hoisted form is 60 instructions vs 50, carries a runtime
overlap check between `out` and `cfg` before the vector loop, and reloads `scale` / `bias` per
iteration in the scalar tail. Timing indistinguishable at 4096 (0.058–0.075; the vector loop
dominates). → hoisting is a readability choice through references; through raw pointers it removes
an alias check. Refutes the unmeasured "hoist config into locals" discipline as a general rule.

**EV-53 — `core::ops::ControlFlow` + `?` in a body closure vs manual `continue`.** (A, L)
`fl_manual = fl_flow`; `size_of::<ControlFlow<()>>() == 1`, `Option<ControlFlow<()>>` == 1.
→ ZERO-COST.

**EV-54 — error-type widths.** (L) A `RhiError`-shaped `Copy` enum (six variants, two
`&'static str` payloads): 24 B; `Result<(), E>` = `Result<u32, E>` = `Result<u64, E>` = 24. A
`String`-payload enum: 24; `Result<(), Box<dyn Error>>`: 16 (plus heap + vtable). A `#[repr(u8)]`
three-variant tag: 1; `Result<(), E>` = 1, `Result<u32, E>` = 8, `Result<u64, E>` = 16. → the
payload sets the width every `?` moves; a `&'static str` payload costs 16 B over a bare tag.

**EV-55 — `#[expect(lint, reason = "…")]`.** (B) A fulfilled expectation (the function IS dead)
is silent; an unfulfilled one (the function is called) warns `unfulfilled_lint_expectations` with
the reason printed in the note. Stable since Rust 1.81. → a self-retiring `#[allow]`; compile-time
only.

**EV-56 — `OnceLock::get()` per element vs hoisted to the loop entry.** (A) `pub static SCALE:
OnceLock<u32>` with an exported setter so the state is opaque. FIRST ATTEMPT RECORDED: with a
private, never-`set` static, LLVM proved `get()` always `None` and folded BOTH functions into an
unconditional `expect_failed` — the probe measured nothing and the vector ops first counted
belonged to the next function in the file. Corrected: per element, the loop body is
`movl SCALE(%rip); testl; jne <cold>` then the multiply — the state word is reloaded and tested
every iteration, 8 instructions per element, zero vector ops. Hoisted
(`let s = *SCALE.get().expect(..)` above the loop): the normalised diff against the
plain-parameter function is exactly one state load, one compare, one cold `expect_failed` block
and the frame setup; the loop itself is the vectorised plain loop (`pmuludq` / `movdqu`, 8 vector
ops). → COSTS per element; hoisting restores the plain loop.

**EV-57 — `clippy::undocumented_unsafe_blocks` comment placement.** (B) Six placements under
`#![warn(clippy::undocumented_unsafe_blocks)]` on rustc 1.97.1's clippy: `// SAFETY:` directly
above — accepted; ONE BLANK LINE between comment and block — accepted; `/* SAFETY: */` block
comment — accepted; `// SAFETY:` with a `//   ` continuation line — accepted; `let v = // SAFETY:
…` trailing on the `let` line that opens the block — accepted; a `debug_assert!` STATEMENT between
comment and block — REJECTED ("unsafe block missing a safety comment"). → the lint requires the
comment to be the last non-whitespace thing before the block; "no blank line" is house form, not
the lint (ERG-20).

**EV-58 — the `extend_exact` per-element release `assert!` vs a bound `zip` + post-check vs
trusting `len()`.** (A, T) Two iterator sources — the `spawn_batch_command.rs` shape
`(start..start + n).map(Id)` and a slice `.iter().copied()` — and three fill forms: (A) the
`VmColumn::extend_exact` shape (grow once from `len()`, `assert!(count < additional)` before
each raw write, a post-check for under-yield); (B) fill through `spare.iter_mut().zip(&mut
iter)` so the SLICE bounds the writes, one post-check for count and exhaustion; (C) the refused
form that trusts `len()` with no per-element check — the codegen floor. Range source: all three
compile to the SAME vectorised main loop (two `movdqu` stores + `paddd` per 8 elements, 9 vector
ops in each function); the per-element assert compare does NOT appear in the vector loop — LLVM
proved it from the trip count — and survives only in the ≤ 7-element scalar remainder as one
`cmpq; je <panic>` per element, where (B)'s `zip` carries an equivalent two compares. Whole
function: (A) 68 instructions with 3 cold panic sites, (B) 77 with 2, (C) 52 with 1 — the
deltas are setup and cold blocks. Slice source: (A) and (B) vectorise identically (4 vector ops
each; neither becomes a `memcpy` call). Timing at n = 4096, three process runs: (A)
0.070–0.076, (B) 0.068–0.070, (C) 0.067–0.070 ns/elem — indistinguishable within the run-to-run
swing. → the per-element release `assert!` is ZERO-COST in the vectorised body for the tree's
two production iterator shapes and costs one compare per remainder element; the bound `zip`
shape buys nothing over it. NOT covered: an opaque iterator whose `next()` LLVM cannot see
through — there the compare stays, beside a per-element call that dwarfs it. Resolves the
index's OPEN 4 for `VmColumn::extend_exact`'s callers. **Closed by EV-60:** the opaque-iterator
case was built and timed identically.

## Rows added by the editorial pass (2026-09-02)

**Where.** EV-59 is the in-tree finder's lab (`D:/tmp/costlab`, gnu 1.97.1, edition 2024,
`codegen-units = 1`, `-C target-feature=+avx2`); EV-60 is the runtime-cost guard's probe
(`D:/tmp/guardprobe`, gnu 1.97.1, the SHIPPED profile: `codegen-units = 16`, SSE2, `panic =
unwind`, timings best-of-7 × 2000 reps at n = 4096 over 64-byte `#[repr(C, align(16))]`
components, every figure reproduced across three process runs); EV-61..EV-70 are the editor's lab
(`D:/tmp/editlab`, gnu 1.97.1, edition 2024, built at `codegen-units = 1` for ICF and re-built at
the shipped 16 — every identity below held at both). Nothing under `crates/` was built or linked.
Rows EV-02, EV-21, EV-22 and EV-23 above carry a SUPERSEDED / QUALIFIED note pointing here;
EV-64 and EV-67 carry a SCOPED / NOT ADOPTED note pointing at the second revision's rows
(EV-71 … EV-73) below.

**EV-59 — the finder's lab, summarised.** (L, A, T) Six ICF aliases: `is_null_rust = is_null_c`
(`Option<NonZeroU64>` handle vs bare `u64 == 0`), `newer_rust = newer_c` (a `TickWindow` pair
struct vs two `Tick` parameters — 8 bytes, same register slots), `size_rust = size_c`
(`ByteStride × RowCount` vs `u32 × u32`), `slot_eq_r = slot_eq_c` (a `Generation` newtype vs
alias), `mw_r = mw_c` (`Polarity` vs `bool`), `run_manual = run_guard` (a non-unwinding body).
Layout: `ThreadRole` 4 = `Cell<ThreadRole>` 4; the view enum 8; `Option<usize>` 16 vs
`Option<NonZeroU32>` 4 vs `Option<NonZeroUsize>` 8; `Option<IslandId>` 4; `Option<LiveEntity>` 16
with `offset_of!(unit_index) == 8` and `zeroed().is_none()`; `Polarity` 1, `Option<Polarity>` 1;
`Result<(), FieldlessEnum>` 1; `LoadEntityMap` flag form 32 vs typestate 24; `SdfOp` 4,
`Option<SdfOp>` 4. Asm: a two-domain CSR walk over newtyped vs raw slices label-normalised
IDENTICAL (124 instructions, vectorised); `lane` over the view match 9 vs 10 with the same two
compares; `get` over the typestate vs the flag IDENTICAL (41); `Column` read over `Option<NonNull>`
vs raw IDENTICAL (24); `layout_checked(RegisteredId)` with `& (N-1)` vs `get_unchecked` IDENTICAL
(2); `mk_r(SdfOp)` vs `mk_c(u32)` IDENTICAL (3); `combine` as `match` vs `if op ==` chains same
opcode multiset, enum one byte shorter; `Option<IslandId>` guard loop one fewer prologue
instruction; **`Option<LiveEntity>` lookup 25 vs 23 instructions** (layout-free, not codegen-free
— LLVM merges the two `u32` payload words into one `u64` and shifts). Timing: a 2^20-entry sparse
array probed 2^20 times at golden-ratio stride — `Option<usize>` 27.20 ms, `Option<NonZeroU32>`
10.67 ms (2.55×, working set 16 MiB → 4 MiB). → the sentinel, newtype, typestate and
`Option<NonNull>` conversions are ZERO-COST; `SparseMap`'s current spelling COSTS runtime; the
tagged-union conversion is layout-free and measured per site.

**EV-60 — the runtime-cost guard's probe, summarised.** (A, T at the shipped profile) Held or
reproduced louder: `Box<dyn Iterator>` on a 64-byte column **2.8×** (53 instructions / 9 calls / 6
vector ops vs 35 / 0 / 13); `dyn` per element **2.5–3×** (1.35–1.70 vs 0.535–0.544 ns/elem; 2
vector ops vs 88); a `match` INSIDE the loop vs hoisted 0.536–0.541 vs 0.540 — LLVM unswitches, the
hoist is readability; associated-`const` arm deletion 0 diff lines on a 64-byte component with a
mask load in the dead arm; `ControlFlow` + `?` through a closure capturing two mutable locals =
ICF alias with the `bool` flag; a two-variant enum ONE instruction shorter than `bool` (`movzbl`
gone); a 64-byte `#[repr(transparent)]` wrapper by value = ICF alias with the plain form; a
64-byte by-value `Copy` receiver instruction-identical to `&self`, **no `memcpy` anywhere**
(refutes the previous ERG-30 mechanism); `zip` over 64-byte columns 69 / 39 vector ops / 0 panic
sites vs index 80 / 23 / 2, hoisted release `assert!` 76 / 40 / 1 and fastest; a per-element
release `assert!` on an OPAQUE iterator 1.604–1.646 vs 1.582–1.654 — indistinguishable (closes
EV-58's open case); `#[cold] -> !` helper 8 vs 18 hot instructions; `OnceLock::get()` per element
1.6×; `SeqCst` store `xchgq` vs `Release` store `movq`; `..Cfg::DEFAULT` = `..Default::default()`
alias on POD; `IntoIterator for &T` alias; `&raw const` = cast alias. Did NOT reproduce:
`get_unchecked` under a proof — asm 22 → 48 with the panic tail gone, time 0.765–0.791 vs
0.717–0.779 (`f32`, 16 KB) and 0.793 vs 0.780 (64-byte, 256 KB) — supersedes EV-02 / EV-23 as a
speed claim; `split_at_mut` vs raw pointers — both `addps`-vectorised, 0.078–0.085 vs 0.082–0.085
(`f32`) and 1.062 vs 1.083–1.102 (64-byte) — supersedes EV-22; `chain` **3–8 %** at column scale
(0.800–0.888 vs 0.780–0.801), not 1.7× — qualifies EV-21; the `*Assign` operator form 61 / 46
vector ops / 1.109–1.132 vs by-value `*m = *m + k` 55 / 41 / 1.073–1.095 — the by-value form is
SMALLER, refuting the previous ERG-16 reason.

**EV-61 — `if const { T::FLAG }` vs `if T::FLAG` vs no arm.** (A, B) `ic_plain_table`,
`ic_const_table` and `ic_none` (a hand-written loop with no arm) emitted as ONE symbol
(`ic_plain_table = ic_const_table`, `ic_none = ic_const_table`) — at `codegen-units = 1` AND at
the shipped 16; 39 instructions, `paddd`-vectorised. The `= true` instantiations: 35 instructions,
0 normalised-diff lines between the two spellings. `if const { self.flag }` → `error: attempt to
use a non-constant value in a constant`. → ZERO-COST; the `const { }` form additionally refuses
the runtime-state refactor at compile time (ERG-31).

**EV-62 — `Drop` guard vs manual epilogue, `panic = unwind`.** (A) Body an opaque `fn()` pointer
(may unwind): `run_manual` 20 instructions, `run_guard` 27 — the normal path is IDENTICAL (the
same 20), the delta is exactly the cleanup landing pad: `movq; callq drop_glue::<Guard>; movq;
callq _Unwind_Resume; ud2; callq panic_in_cleanup; ud2` plus a `GCC_except_table` entry. Body a
plain arithmetic statement (cannot unwind): `run_manual_nounwind = run_guard_nounwind` (ICF
alias) at 1 and 16 units. → ZERO-COST on the normal path; the landing pad IS the thing bought
(ERG-43).

**EV-63 — atomic orderings on x86-64.** (A) `store(v, SeqCst)` → `xchgq %rdx, (%rcx)` (a
locked read-modify-write); `store(v, Release)` and `store(v, Relaxed)` → `movq %rdx, (%rcx)`.
`load` at `Relaxed` / `Acquire` / `SeqCst` → the identical `movq (%rcx), %rax`. `fetch_add(1, _)`
at `SeqCst` and `Relaxed` → the identical `lock xaddq`; with the result unused → `lock incq` at
both. `compare_exchange` vs `compare_exchange_weak` in a retry loop → the identical `lock
cmpxchgq; jne`. → a `SeqCst` STORE costs a bus lock; everything else is ordering-independent
on this ISA; the strong/weak CAS split is portability, not x86 cost (ERG-45).

**EV-64 — the worker-role sentinel: stored transparent newtype + view enum vs two constants
compared by hand.** (L, A) `size_of::<ThreadRole>() == size_of::<Cell<ThreadRole>>() == 4`;
`size_of::<Role>()` (view) == 8; `size_of::<NaiveRole>()` (`enum { Worker(u32), Dispatcher,
Unattached }` as the STORED type) == **8**; `size_of::<Option<WorkerId>>()` == **8** (no niche on a
plain `u32`). `lane_r` (the view match) 9 instructions vs `lane_c` 10, the same `cmpl $-1` /
`cmpl $-2` chain, one fewer register move. A table-indexing `wake` over a `&[u64]`: `wake_c` vs
`wake_r` 0 normalised-diff lines (16 each; LLVM merged both sentinel compares into one `cmpl $-3;
jbe` in BOTH forms — the finder's "no frame, no panic path" claim did not reproduce on a
slice-typed table and is not carried). → the stored + view split is ZERO-COST; the naive stored
enum COSTS a word (ERG-04, REF-34). **SCOPED (2026-09-02, revision 2, EV-71): this row measured a
`u32` payload. The worker id's domain is `< 64`, so a `#[repr(u8)]` enum with a `u8` payload is
2 bytes and is the adopted stored form for THIS case; the split remains the answer for a
payload that needs its full width (ERG-04 shape 2b).**

**EV-65 — `mem::take` / `mem::replace` to split a borrow.** (A) `tk_take` (take the `Vec` field
out, call a helper that also reads `self`, put it back) and `tk_reborrow` (the conflict-free
reference form): the SAME 5 instructions, reordered — LLVM deleted the move-out and move-back
entirely. `tk_replace_big` (`mem::replace` on a `[u8; 4096]` field): two 4095-byte `memcpy`
calls, a 4136-byte frame and a `___chkstk_ms` probe. → ZERO-COST on a `Vec` / word-sized field;
COSTS two memcpys on a large inline array (ERG-43).

**EV-66 — scoped-closure accessor vs a free-lifetime reference.** (A, B) `sc_scoped`
(`c.with_pool(|p| …)`, `FnOnce(&Pool) -> R`) and `sc_direct` (`c.pool_ref::<'a>()` then the same
arithmetic) → ICF alias `sc_scoped = sc_direct` at 1 and 16 units. A closure that returns its
argument (`with_pool(p, |x| x)`) → `error: lifetime may not live long enough … returning this
value requires that '1 must outlive '2`. → ZERO-COST; the escape is refused by the signature
(ERG-06 shape 2).

**EV-67 — SAFE index under a range-narrowing accessor vs `unsafe` `get_unchecked`.** (A)
`layout_unchecked(usize)` (`unsafe fn`, `get_unchecked`): 3 instructions. `layout_checked(RegisteredId)`
(safe, `&LAYOUTS[id.0 as usize & 511]`): 4 — one `andl $511, %ecx` more, NO compare, NO
`panic_bounds_check`. `layout_nomask` (safe, `&LAYOUTS[id.0 as usize]`, no mask): 13 with `cmpl
$511; ja` and a panic path. `layout_500` (safe, `% 500` on a non-power-of-two table): 8 — a
multiply-shift-subtract, no branch. → the mask is load-bearing for the COUNT; the safe spelling
costs one `and` against the `unsafe fn` and deletes the fn's contract. **NOT ADOPTED (2026-09-02,
revision 2): the mask clause was withdrawn on review — on a valid id it does nothing, on an
invalid one it converts a bounds panic into a silent read of the wrong row (ERG-26 case 2), the
4-vs-13 figure is an instruction count (which this ledger's reading rule refuses as a cost), and
EV-60 measured no time delta from the check. The ADOPTED form is this row's `layout_nomask`: a
plain `[]` under the proof-carrying id, 13 instructions with a never-taken branch (ERG-03,
REF-40).**

**EV-68 — `-> bool` vs `-> Result<(), FieldlessEnum>`.** (L, A) `size_of::<Result<(),
RowOutOfBounds>>() == 1 == size_of::<bool>()`. `remove_c` and `remove_r`: the same 8
instructions differing only in `setb` / `setae`. The bundle loop: `bundle_c` (`ok &= remove_c(..)`)
27 instructions, `bundle_r` (`remove_r(..)?`) 27, the latter with `testb %al; jne` exiting on the
first `Err`. → ZERO-COST; the `Result` form is `#[must_use]` by construction and stops at the
first failure (ERG-28).

**EV-69 — `Column` with a null-sentinel `*mut u8` vs `Option<NonNull<u8>>`.** (L, A) Both:
size 16, align 8, `offset_of!(stride) == 8` (the file's existing gates pass unchanged).
`zeroed_is_none()` → `movb $1, %al; retq` (folded to a constant `true`). `read_c` (bounds-checked
table index, `is_null` test, scaled read) vs `read_r` (`let base = col.ptr?`): 22 instructions
each, 0 normalised-diff lines. → ZERO-COST (ERG-22).

**EV-70 — `split_at_mut` vs raw pointers off one base, `f32` halves, rustc 1.97.1.** (A)
`s24_slice`: 40 instructions, 2 `addps`, 1 `addss`, 6 `movups`. `s24_raw`: 34 instructions, 2
`addps`, 2 `addss`, 12 `movups` — BOTH vectorised, behind a `cmpq $16` length guard. Timing per
EV-60: parity. → supersedes EV-22's "scalar fallback"; the slice form is at worst equal (ERG-24,
REF-19).

## Rows added by the second revision (2026-09-02) — the confirming reader's objections, compiled

**Where.** `D:/tmp/editlab2` (outside the repo), gnu 1.97.1, edition 2024, opt-level 3, no LTO,
`panic = unwind`, built at `codegen-units = 1` and re-built at the shipped 16 (the crate is one
unit either way; every identity held at both). Bodies extracted from the `.s`, directives
stripped, labels normalised, diffed. Nothing under `crates/` was built or linked.

**EV-71 — the worker-role sentinel, with the payload RIGHT-SIZED.** (L, A, B) The worker id is
`< MAX_WORKERS == 64` (`thread_pool.rs`, pinned against `LANE_WORKER_MAX` by a `const _`), so
the payload fits a byte. Gates: `#[repr(u8)] enum ThreadRole { Worker(WorkerId /* u8 */),
Dispatcher, Unattached }` — size **2**, align 1 (RFC 2195: tag byte + payload byte);
`Cell<ThreadRole>` 2; `Option<ThreadRole>` 2 (the tag has spare niches); `Option<WorkerId>` 2 (a
plain `u8` has no niche); `ThreadLabels { role, lane: Lane(u16) }` 4 and `Option<ThreadLabels>`
4; the default-repr form also 2 (not pinned); `enum { Worker(u32), .. }` **8**; `enum {
Worker(NonZeroU32), Dispatcher, Unattached }` **8** (one niche = one spare variant); the same
with only `Dispatcher` 4; `#[repr(u8)] enum { Worker(u16), .. }` 4; the previous draft's
transparent `ThreadRole(u32)` 4 and its view enum 8. Asm: `lane_r` (an exhaustive `match` over
the 2-byte enum) 10 instructions vs `lane_c` (the sentinel chain) 10 — `testb %cl,%cl` / `cmpl
$1` on the tag byte replace `cmpl $-1` / `cmpl $-2`, and the `movl` loads become `movzbl`;
`lane_t` (transparent + view) 9. `wake_r` (table index under `match`) 16 vs `wake_c` 16, 8
normalised-diff lines — the tag test replaces the merged `cmpl $-3; jbe` sentinel compare, the
`panic_bounds_check` path is present in both. `ThreadRole::lane(self) -> Option<Lane>` derives
the diagnostics lane for `Worker` and `Dispatcher`; `Unattached` returns `None` because the tree
documents that the lane there is host / spare / unclaimed and is SAVED, not derived. → the
right-sized data-carrying enum is the stored form: smaller than the sentinel, same instruction
count, no encoding behind an accessor (ERG-04 shape 2a); EV-64's split stays for full-width
payloads.

**EV-72 — `SharedPtr`: `Copy` handle + free-lifetime `as_ref` vs a non-`Copy` handle consumed
by `complete(self)`.** (A, B) `body_c` (`let shared = unsafe { sp.as_ref() }; if failed {
shared.capture_panic(p) } shared.complete_task()`) and `body_r` (`if failed {
sp.capture_panic(p) } sp.complete()` with `complete(self)` consuming a `NonNull`-carrying,
non-`Copy` handle; `complete_task` = `Thread::unpark` + `fetch_sub(1, AcqRel)`): **22
instructions each, 0 normalised-diff lines** at codegen-units 1 and 16 — a move of one pointer
is the same machine code. Behaviour: `sp.complete(); sp.capture_panic(1);` → `error[E0382]:
borrow of moved value: sp … SharedPtrR::complete takes ownership of the receiver self`. → the
"MUST NOT be dereferenced after this line" comment becomes a compile error at zero cost (ERG-06
shape 1).

**EV-73 — `claim_one_idle` with `IdleWord` / `ExcludeMask` / `WorkerId` vs `u64, u64, u32`.**
(A, B) `claim_c` (the tree's body verbatim: `observed & !exclude`, rotate, lowest bit, `% 64`,
`compare_exchange_weak(observed, new, AcqRel, Acquire)`) and `claim_r32` (the same body over
`IdleWord::claimable(ExcludeMask)`, `IdleWord::without(WorkerId32)` and a `u32`-payload id):
**ICF alias `claim_r32 = claim_c`** at 1 and 16 units. `claim_r` with EV-71's byte-sized
`WorkerId` returned as `Option<WorkerId>`: 18 instructions vs 19 (`addb` / `andb` for `addl` /
`andl`, one fewer `movl $0`). A caller loop (`load(Acquire)`, claimable test, claim, index the
worker table): `unpark_loop_c` 43 vs `unpark_loop_r` 43, the diff being `movzbl` for `movl` and
the callee name. `IdleWord` carries NO `BitAnd` — its only operations are `claimable` and
`without`. Behaviour: `claim(i, e, o, s)` (transposed) → `error[E0308]: arguments to this
function are incorrect — expected ExcludeMask, found IdleWord / expected IdleWord, found
ExcludeMask`; `claim(i, o.claimable(e), e, s)` (the folded call the doc says hangs) →
`error[E0308]: mismatched types — expected IdleWord, found u64`. → ZERO-COST; both documented
misuses stop compiling (ERG-02).

## Rows added by the third revision (2026-09-02) — the confirming reader's second set of objections, compiled

**Where.** `D:/tmp/editlab3` (outside the repo), gnu 1.97.1 confirmed (`rustc 1.97.1 (8bab26f4f
2026-07-14)`), edition 2024, opt-level 3, no LTO, `panic = unwind`, an rlib plus a binary that
calls every probe, built at `codegen-units = 1` and re-built at the shipped 16 — every count and
diff below held at both. ⚠️ Method note: this lab emitted NO `.set` alias line for any pair,
including the byte-identical `mark_c` / `mark_r` bodies, at either unit count and for both crate
types — LLVM's merge pass does not fire on every crate shape, so an absent alias line is not
evidence of difference (the profile paragraph already says this for 16 units; it holds at 1 as
well). The identity evidence for these rows is the label-normalised diff, which the method
section ranks next to the alias. Nothing under `crates/` was built or linked.

**EV-74 — a `[lo, hi)` pair as `(usize, usize)` vs `core::ops::Range<usize>`.** (L, A, B)
Layout: both 16 bytes, align 8; `Option<_>` 24 for both (no niche on `usize` either way);
`Range<u32>` 8 = `(u32, u32)`. `span_c` (`let (start, end) = span; for i in start..end { s +=
xs[i] }`) vs `span_r` (`for i in span { s += xs[i] }`): 17 instructions each, 0 normalised-diff
lines beyond the panic-location constant — the per-element `cmpq %rdx, %r8; jae` bounds check is
in BOTH. `span_slice` (`for &x in &xs[span]` — the form only the `Range` can spell): 23
instructions, the bounds check hoisted to two compares BEFORE the loop and none inside (none of
the three vectorise: `f32` addition is not associative). `span_len_c` (`end - start`) 3
instructions, wrapping; `span_len_r` (`Range::len`) 4 with a `cmovae`, saturating at 0 for a
reversed range. Behaviour, recorded honestly: `f(end..start)` COMPILES and returns `len() == 0`;
the `Range` does NOT refuse a reversed pair — what it buys is the order stated at construction
(`start..end`), `len` / `is_empty` / `contains` with saturating semantics, and the slice-index
form. → ZERO-COST; a legibility and slice-reachability change, not a compile-time refusal
(ERG-02, a SHOULD).

**EV-75 — a sink reachable by construction: push through a raw index vs through a handle minted
only by registration.** (A, B) `push_c(inner, idx: usize, v)` (`inner.queues[idx]`) vs
`push_r(inner, q: Scanned, v)` where `Scanned(usize)` has a private field and its ONLY
constructor is `Inner::register`, which pushes the queue AND its index into the scan set in one
operation: 20 instructions each, 0 normalised-diff lines beyond the panic-location constant, at 1
and 16 units. Behaviour: `Scanned(3)` from outside the module → `error[E0423]: cannot
initialize a tuple struct which contains private fields`. → ZERO-COST; a push into a queue no
scan set contains has no value to be spelled with (ERG-03 shape 2).

**EV-76 — the idle-set shift over a `u32` + `debug_assert!` vs a `WorkerId(u8)` bounded `< 64`
at the mint.** (A) `mark_c` (`debug_assert!(id < 64); idle.fetch_or(1u64 << id, Release)`) vs
`mark_r` (`idle.fetch_or(w.bit(), Release)`, `bit() = 1u64 << self.0`): 6 instructions each,
**0 diff lines** (`shlq %cl; lock orq`); `unmark_c` / `unmark_r` likewise 0 diff lines (`rolq
%cl` of `$-2`; `lock andq`). `wake_c(&[u64], u32)` vs `wake_r(&[u64], WorkerId)` (the worker
table index): 11 = 11, the diff being `movzbl` for `movl` and the panic constant — the `[]`
bounds check is present in BOTH, because the type proves `< 64`, not `< worker_count`. In release
the `debug_assert!` is gone and `shlq` masks its count to six bits: `1u64 << 70` marks bit 6 —
the wrong worker — silently; the bounded mint makes that value unrepresentable. → ZERO-COST;
closes the atomic-wrapper half of OPEN 7 (ERG-03, ERG-45).
