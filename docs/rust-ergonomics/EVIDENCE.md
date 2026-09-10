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

**Profile — the SHIPPED one first; then every block of rows that was taken at a different one.**
(Rewritten 2026-09-03 by the re-verification pass; the paragraph that stood here is quoted below
where it was wrong.) `cargo build --release` on this checkout (`feat/threadpool-ke16` at
`bfc5b41f`) IS: edition 2024 · `opt-level = 3` · **`codegen-units = 16`** (cargo emits no flag;
rustc's non-incremental default) · **`lto = false`** (`-C embed-bitcode=no` on the rustc line) ·
`panic = unwind` · `debug = 0`, `debug-assertions` off, `overflow-checks` off, `incremental` off ·
`strip = debuginfo` (cargo 1.97's default when `debug = 0`, so the release binary is already
stripped without anyone asking) · **`-C target-cpu=x86-64-v3`** (the three `[target.x86_64-*]`
tables of `.cargo/config.toml`, live since `cced895a` on 2026-09-02, mirrored in CI's `RUSTFLAGS`
because `RUSTFLAGS` REPLACES config rustflags, and guarded by `tests/isa_baseline_census.rs`) ·
`rustc 1.97.1 (8bab26f4f 2026-07-14)`, `stable-x86_64-pc-windows-gnu`. Established by grepping
every manifest — there is exactly ONE `[profile` table in the whole workspace, `[profile.bench]
codegen-units = 1` in the root `Cargo.toml`; no `[profile.release]`; no `opt-level` / `lto` /
`panic` / `debug` / `overflow-checks` / `incremental` / `strip` key in any of the 29 manifests; no
`CARGO_PROFILE_*` in any CI job — and CONFIRMED from the rustc command line of a real
`cargo build --release -p boyko-math -v` (EV-115). ⚠️ **`cargo bench` and `cargo build --release`
are therefore DIFFERENT codegen configurations on this tree, by exactly one axis** (the bench
profile's single unit): a number taken with `cargo bench` and reported as "the shipped profile" is
wrong by that axis, and the profile investigation neutralised it explicitly (EV-118).

⚠️ **Rows EV-01 … EV-58 were produced at `codegen-units = 1`, and several magnitudes in the ledger
were L1-resident artefacts.** That is the correction this pass exists to make visible. LLVM's
identical-code folding runs only WITHIN a unit, so at 16 units two byte-identical functions may
produce no alias line purely by partitioning — **the absence of an alias is not evidence of
difference at the shipped profile, and (EV-122) the PRESENCE of one in a row taken with a bare
`--emit=asm` is single-unit evidence whatever the profile says.** And a timing whose working set
fits L1 measures the L1, not the technique: every per-element cost in this ledger vanishes into
memory latency once the column is past the LLC (third reading rule, below). Where the rows depart
from the shipped profile, by block:

- **EV-01 … EV-58** — `codegen-units = 1`, bare x86-64 (SSE2), single-point timings mostly at
  n = 4096 `f32` = 16 KiB (L1). Identity results are ISA-independent; every absolute ns figure is
  SSE2 and does not transfer; every ICF alias is a one-unit observation.
- **EV-59** — `codegen-units = 1`, `-C target-feature=+avx2` — an ad-hoc feature list, NOT the
  `x86-64-v3` level the tree ships (which also selects the scheduling model). One timing point
  (2^20 entries, exactly the LLC boundary) that the re-verification could not reproduce on this
  box under load; re-scaled in the row.
- **EV-60** — `codegen-units = 16`, SSE2, ONE fixture: n = 4096 × 64-byte `#[repr(C, align(16))]`
  components = 256 KiB — an L2 point at the TOP of the engine's component-size range (EV-125).
  Every ratio it reports reads "at L2, at 64 bytes, on one column"; its per-claim verdicts are now
  carried IN the rows it qualified (EV-02, EV-13, EV-14, EV-21, EV-22, EV-23, EV-56).
- **EV-61 … EV-76** — built at `codegen-units = 1` and "re-built at the shipped 16" — ⚠️ **the
  flag the re-build was taken with is NOT recorded**, and a bare `--emit=asm` silently forces ONE
  unit (EV-122). A 16-unit claim in that range is checkable only where the 2026-09-03 pass re-took
  it with an EXPLICIT `-C codegen-units=16` and a forced unit split: EV-59's six aliases, EV-61,
  EV-62, EV-69's read-body pair and EV-70's `f32` pair (through EV-22) — all hold. EV-63 … EV-68
  and EV-70's 64-byte sentence carry their "held at 16" as unverified single-unit evidence.
- **EV-77 … EV-95** — an explicit `codegen-units = 16` in the lab's own `[profile.release]`,
  `x86-64-v3`, every timing under load and printed three times.
- **EV-96 … EV-114** — explicit `codegen-units = 16`, re-checked at 1, `x86-64-v3`.
- **EV-115 … EV-125** (the 2026-09-03 re-verification and the profile investigation) — the
  shipped profile, spelled per row, with the unit count NAMED on every identity, THREE working-set
  points on every timing, and the override variable named wherever a configuration was varied.

*The ISA sentence that stood here until 2026-09-02, kept struck through rather than deleted:*
~~"The SSE2 baseline IS the shipped ISA: no `.cargo/config.toml`, `Cargo.toml` or CI file sets
`target-cpu` / `target-feature` … `CLAUDE.md` names AVX2 as the baseline; the build configuration
does not enable it."~~ False since `cced895a` (2026-09-02 12:16, "ship the ISA baseline the
project has declared since it began"; amended by `af6e7043`): `.cargo/config.toml` sets
`rustflags = ["-C", "target-cpu=x86-64-v3"]` for `x86_64-pc-windows-gnu`, `-msvc` and
`-unknown-linux-gnu` (owner ruling "AVX2 by default, all modern processors support it", FMA
included with the measured "Rust does not contract an explicit `mul` + `add`" note). **AVX2 IS the
shipped ISA, `boyko_physics`'s `cfg(target_feature = "avx2")` solver arms DO compile in (EV-116),
and the setting is worth 1.97× on the AVX2 colored solve (EV-117).** The 2026-09-03 brief that
commissioned this pass repeated the stale sentence; all three lenses caught it before measuring.

**Where.** A scratch Cargo workspace under the system temp directory (rows EV-01..EV-41), a
single-file `rustc --emit=asm` probe in the session scratchpad (rows EV-42..EV-44), a second
scratch crate in the session scratchpad with a counting `#[global_allocator]` (rows EV-45..EV-55,
same profile and toolchain, `rust-toolchain.toml` pinning the gnu channel), a third
(`lintprobe`, same pinning and profile) for rows EV-56..EV-57, and a fourth (`eeprobe`, same
pinning and profile, `cargo build --release` with `--emit=asm`) for row EV-58. Rows EV-96 … EV-107
came from a FIFTH lab (`erglab5`, outside the repository, `rust-toolchain.toml` pinning the gnu
1.97.1 channel, `.cargo/config.toml` setting `-C target-cpu=x86-64-v3`, `[profile.release]`
`opt-level = 3` / `codegen-units = 16` / no LTO — the SHIPPED profile — with a byte-identical
sibling at `codegen-units = 1` for the ICF re-check), a `#![no_std]` probe crate holding a VERBATIM
copy of three `boyko_utils` module trees (EV-105), and two single-file `rustc` probes on the
pinned nightly (EV-107). Nothing under `crates/` was built, modified or linked into any probe.

**Four methods**, named per row:

- **L (layout)** — `const _: () = assert!(size_of / align_of / offset_of …)` compiled in the
  release profile. Compiling IS the proof; a false claim is an `E0080` build failure.
- **A (assembly)** — `--emit=asm`, then reading the `.s`. At `codegen-units = 1` LLVM performs
  identical-code folding and emits an explicit alias line `sym_a = sym_b` when two functions have
  byte-identical bodies. **An ICF alias is the strongest identity evidence available** — stronger
  than a diff. Where functions did not fold: a label-normalised diff, an opcode histogram, or a
  per-element steady-state instruction count.
  ⚠️ **Instrument defect, measured 2026-09-03 (EV-122).** `cargo rustc --release -- --emit=asm`
  with NO explicit `-C codegen-units` silently emits ONE unit — 1 `.s` file and 13 alias lines on
  a crate that `cargo build --release` compiles into THREE `cgu.*.rcgu.o` members — and a
  single-file `rustc --emit=asm` prints `warning: resetting to default -C codegen-units=1` and
  ignores a requested 16. So an alias observed with a bare `--emit=asm` is a one-unit observation
  whatever the profile says, and every identity in this ledger names the unit count it was taken
  with. At an EXPLICIT 16 the partitioner puts the two halves of a probe pair in different units
  (verified by `.file "…cgu.NN"` markers), the alias line is ABSENT and the bodies are still 0
  normalised-diff lines: at the shipped profile an absent alias is partitioning, a present one is
  a coin flip, and the evidence is the diff — or the CALLEE MAP, because EV-02's sequential pair
  merged so completely that one symbol has no body at all and its 17 call sites were rewritten to
  the other, which no alias grep finds.
- **T (timing)** — hand-rolled harness, best-of-7 × N reps after warm-up, `black_box` on operands
  and result, ns per element; every figure reproduced across ≥ 2 independent process runs.
  **Since 2026-09-03, additionally:** THREE working-set points per figure spanning L1 (16 KiB),
  L2 (256 KiB) and beyond the 16 MiB LLC (64 MiB), sized against the tree's own shapes (EV-125);
  ≥ 3 separated process runs with the median and the full [min..max] printed; CPU load and the
  count of concurrent `rustc` / `cargo` processes sampled before and after every run; a
  contaminated run NAMED beside its verdict, never dropped; and a ratio inside the measured noise
  band (fourth reading rule) reported as "no claim", never medianed into one.
- **B (behaviour)** — two-crate workspaces for cross-crate properties, deliberate compile-fail
  probes for auto-trait and lint claims, one runtime probe printing a panic `Location`.
- **C (allocation count)** — a `GlobalAlloc` wrapper over `System` counting `alloc` / `dealloc` /
  `realloc` with relaxed atomics; deltas read around the call under test, the result dropped
  outside the counted region. This is the instrument EV-29 said was missing.
- **S (type sizes)** — `cargo +nightly rustc --release -- -Zprint-type-sizes`, sorted by size.
  The instrument that FINDS the oversized type a method-L gate then pins; added by the second
  sweep (EV-95), with its nightly named because a floating nightly is never acceptable here:
  `nightly-x86_64-pc-windows-gnu 1.100.0-nightly (8925ea358 2026-08-20)`. ⚠️ `cargo llvm-lines`
  (which is the instrument behind REF-07's and REF-18's code-size refusals) and `cargo bloat` are
  NOT installed on this box and are NOT method rows: those two refusals rest on hand instruction
  counts, and this ledger says so rather than borrowing an instrument's authority.
- **T (monomorphisation stats)** — `-Zdump-mono-stats=DIR -Zdump-mono-stats-format=markdown` on the
  same named nightly, which writes `<crate>.mono_items.md` with a per-item instantiation count and
  a size estimate. Added by the fourth sweep (EV-112). It needs no install and no channel move, so
  it is the instrument the `cargo llvm-lines` sentence above says is missing — for the COUNT half.
  It has not been run over `crates/`; a claim about REF-07 or REF-18 still owes that run.

**Reading rule.** Instruction count is NOT a cost proxy in either direction: EV-02's faster loop
has MORE instructions (unrolling), EV-22's slower one has FEWER. Any verdict here that rests on a
count alone says so. **Second reading rule (EV-49):** a wall-clock delta over BYTE-IDENTICAL
bodies is code placement, not the technique — the ledger accepts a timing verdict only beside an
asm identity, a body diff, or an allocation count. **Third reading rule (2026-09-03, EV-123):
a timing has no meaning without its WORKING SET.** Every per-element technique cost in this ledger
— a vtable call, a `OnceLock` reload, a short-circuiting `&&`, a bounds check, `chain`'s state
test — is a fixed number of cycles per element, and beyond the LLC each element already costs
40–90 ns of memory latency that hides all of them: measured, `dyn` throughput dispatch is 25× at
L1, 9× at L2 and 1.00× at 64 MiB (EV-14); `OnceLock` per element 16×, 7×, 1.00× (EV-56); `&&`
against `&` 26×, 17×, 1.4× (EV-91). A verdict that holds at one point says so, and the engine's
regime is named beside it: a 64-byte column at `POOL_MIN_ROWS` is 4 MiB (past L2, inside L3),
the 10k-body pyramid's bodies are 640 KiB, and a `par_iter` chunk is 1024–4096 rows = 12–256 KiB
per worker (L1/L2 at every population the engine benches) — so the per-element costs are LARGEST
where a parallel system actually runs and vanish only on a single-threaded walk over ≥ ~250k rows
(EV-125). **Fourth (EV-123): the noise floor is MEASURED over identical machine code before any
ratio is read.** The ICF-aliased pair `f32_seq_unchecked = f32_seq_checked` timed as two variants
differed by 2 % at L1, 6 % at L2 and 25 % beyond LLC on this box under this load; the same source
at the same profile flipped the SIGN of EV-22's `f32` ratio between two binaries. A ratio inside
those bands is not a measurement. **Fifth (EV-124): a `dyn` per-element figure states whether the
concrete type was VISIBLE at the loop** — a `Box<dyn Iterator>` minted and consumed in one function
is devirtualised completely and measures one malloc+free per CALL, which impersonates a per-element
cost at small n and is nothing at large n.

## Rows

**EV-01 — `#[repr(transparent)]` newtype over `u32` vs raw `u32`.** (L, A) 38 layout gates green;
summing loop over `&[Id]` vs `&[u32]` produced the ICF alias `a_raw = a_newtype`. Byte-identical.
Layout half is a Reference guarantee; codegen half is now measured. → ZERO-COST. **RE-VERIFIED 2026-09-03 at the shipped profile (EV-122's method) — HOLDS.** Explicit
ICF alias `a_raw = a_newtype` at codegen-units 16 AND 1 in one module (76 instructions, 29 AVX2
vector ops, 0 calls); with the two halves forced into DIFFERENT units at 16 (`cgu09` / `cgu08`,
verified by `.file` markers) no alias line is emitted and the bodies are 0 normalised-diff lines,
76 = 76. The 38 gates recompiled at `x86-64-v3`; the two-domain CSR walk over newtyped vs raw
slices is 72 = 72, 0 diff, at both unit counts. No time claim is admissible: the two forms are the
same instruction sequence, so a wall-clock delta between them would be placement (EV-49).

**EV-02 — private-field newtype + `get_unchecked` vs a bounds-checked index.**
~~(A, T) 4096 elements. Checked: 7 instr/element (`movzwl, cmpq $4096, jae, xorq, addq, cmpq,
jne`); proven: 2 instr/element because removing the check let LLVM unroll 8×. Timing 0.527–0.547
ns/elem checked vs 0.357–0.359 proven = **1.5× faster**, two runs. → ZERO-COST, removes work.~~
**REFUTED 2026-09-03** (the 2026-09-02 "SUPERSEDED by EV-60" note that stood here is absorbed —
the verdict is in this row). ASM, sequential shape: at the shipped profile the bounds-checked loop
and the `get_unchecked` loop are ONE SYMBOL — ICF alias `f32_seq_unchecked = f32_seq_checked` at
codegen-units 16 AND 1 (35 instructions, 12 vector ops), and on the tree's 40-byte `Transform`
column LLVM merged the pair so completely that `b_seq_unchecked` HAS NO SYMBOL: its 17 call sites
were rewritten to `b_seq_checked` (55 instructions, 27 vector ops, 0 panic sites, the range check
hoisted out of the `0..len` loop). There is no "7 vs 2 instr/element" at `x86-64-v3` and no
codegen delta left to cite. TIME, sequential: parity by construction — the pair doubles as this
box's noise-floor calibration (EV-123): 0.98× at L1, 0.94× at L2, 1.25× beyond LLC over IDENTICAL
code. The delta survives ONLY in the GATHERED shape (`xs[id]` off an index list — a `ComponentPool`
row lookup): checked 22 instructions / 3 vector ops / one `panic_bounds_check` call, unchecked 48 /
12 / none, equal at both unit counts; time 1.28× at 16 KiB (L1), 1.13× at 256 KiB (L2, inside the
6 % band), UNUSABLE beyond LLC (43.9 [36.9..59.3] vs 60.5 [37.0..105.7] ns — spans overlapping by
more than the gap). Three-column query fetch (12 B `Pos` + 12 B `Vel` + the 4-byte per-row `Tick`):
116 instructions / 47 vector ops / 2 panic sites vs 83 / 47 / 0 — the vector body IDENTICAL, the
delta setup and cold tails — and 1.00× / 0.97× / 0.83× at 1024 / 65 536 / 1 048 576 rows (checked
FASTER beyond LLC, i.e. noise). Not one of eighteen paired comparisons shows the 1.5×. → sequential:
IDENTITY, no licence of any kind; gathered: a real codegen delta and ≈13–28 % at L1 ONLY — 256 rows,
below `MIN_ARCHETYPE_FOR_PARALLEL` — gone at the size the engine allocates. Does not license
`get_unchecked` (ERG-03, REF-15, REF-40).

**EV-03 — `const _` layout gates.** (L) 38 gates compiled; they emit no code. The gate
`size_of::<DspBuf<1>>() == 1 + size_of::<usize>()` FAILED with `E0080` — real answer 16, the
`[u8; 1]` is tail-padded to `usize` alignment. The gate caught a wrong belief in the guide's own
sketch within seconds. → COMPILE-TIME ONLY, and load-bearing.

**EV-04 — `Option<Newtype(NonZeroU32)>` niche.** (L, A) `size_of::<Option<SlotIndex>>() == 4`,
align 4; the niche survives two nested `repr(transparent)` layers. A loop over
`&[Option<NonZeroU32>]` and a hand-rolled `0`-sentinel loop over `&[u32]` folded to one symbol
(`v_sentinel = v_niche`). → ZERO-COST; the `Option` IS the sentinel convention. **RE-VERIFIED 2026-09-03 — HOLDS, more tightly than written.** Gates recompiled at the shipped
profile: `Option<SlotIndex>` 4 / align 4 through two nested `repr(transparent)` layers,
`Option<NonZeroU32>` 4, `Option<Option<NonZeroU32>>` 8 (the one-niche-per-type exception EV-102
defends, confirmed again). Codegen at codegen-units 16 and 1: the fold is THREE-WAY —
`v_niche = a_newtype` and `v_sentinel = a_newtype` — the niche loop, the hand-sentinel loop and the
plain unconditional sum are ONE symbol (76 instructions, 29 vector ops), because adding 0 for the
absent case is a no-op LLVM proves away; across a forced unit split, 0 diff, 76 = 76. The runtime
half of the sentinel-versus-niche argument lives in EV-59 and is re-scaled there.

**EV-05 — `Option<NonNull<T>>` is one word.** (L) `== size_of::<usize>() == 8`, also for a
4096-byte pointee. → ZERO-COST (std `Option` representation guarantee). **RE-VERIFIED 2026-09-03 — HOLDS** (profile-independent, re-run anyway): gates compiled at
the shipped profile for `NonNull<u8>` and a 4096-byte pointee. EV-69's companion pair — a `Column`
with a null-sentinel `*mut u8` against `Option<NonNull<u8>>`, bounds-checked / absence-tested /
scaled `read` — is 20 = 20 instructions with 0 normalised-diff lines at codegen-units 16 and 1;
NO alias is emitted at either count because the two carry different panic-location constants,
which is exactly why the diff, not the alias, is the evidence.

**EV-06 — `PhantomData` table: `fn() -> R` vs bare `R`.** (L, B) Both ZST (size 0, align 1).
`ResTag<*mut u8>` with `PhantomData<fn() -> R>` passes `assert_send`; `ResBare<*mut u8>` with
bare `PhantomData<R>` FAILS `E0277` ("`Send` is not implemented for `*mut u8` … required because
it appears within the type `PhantomData<*mut u8>`"). Both 4 bytes. → the table row distinction is
real and load-bearing; bare `PhantomData<T>` as a tag COSTS a wrong auto-trait relation.

**EV-07 — typestate, SMALL payload, consuming transitions.** (L, A)
`size_of::<Builder<OptIn>>() == size_of::<Inner>()`; the chain
`SmallBuilder::new().step1().step2().build()` folded to the literal (`z_small_chain = z_direct`).
→ ZERO-COST. **RE-VERIFIED 2026-09-03 — HOLDS.** Explicit alias `z_small_chain = z_direct` at
codegen-units 16 AND 1; across a forced unit split, 0 diff lines, 2 = 2 (`movl $7, %eax; retq`);
the size gates recompiled. ⚠️ One companion sub-claim of EV-59 that ERG-08 quoted beside this row
is CORRECTED there: the flag-form and typestate `get` are NOT identical at the shipped profile —
typestate 6 instructions, flag 8, the diff exactly the deleted `cmpb $1; jne` — so the typestate is
strictly SMALLER in code (6 vs 8) and in data (24 vs 32 bytes); the zero-cost verdict survives a
fortiori and the word "identical" does not.

**EV-08 — typestate, LARGE payload (4096-byte inline array), 4-state chain.** (A, T) Transitions
`#[inline]`: whole chain compiles to `movl $6, %eax; retq`, ICF-folds with the `&mut self` form,
1.15–1.18 ns. Transitions `#[inline(never)]`: each transition emits `callq memcpy` of 4104 bytes,
caller frame `.seh_stackalloc 8248` plus a `___chkstk_ms` probe, 223–234 ns — **~190× slower**.
→ ZERO-COST only when the chain inlines; COSTS a 4 KiB memcpy per transition when it does not. **RE-VERIFIED 2026-09-03 — HOLDS, larger at the shipped ISA.** `chain_inlined` is THREE
instructions in total (the whole four-state chain over a 4096-byte payload folds to a constant);
`chain_noinline` 33 instructions with 6 calls, each non-inlined transition 10 with one call;
identical at codegen-units 16 and 1. Time, four separated runs: 181.6× at 4 096 chains, 203.7× at
65 536, 228.8× at 262 144 — ranges nowhere near overlapping. Stated honestly, this row has NO
data-scale axis (a fixed 4 KiB stack payload, a per-transition cost): the three points are CALL
COUNTS, and the ns/call figure is flat within 15 % across them, which is itself the confirmation
that it is a per-call `memcpy` and not a cache effect.

**EV-09 — zero-sized capability token as a parameter.** (L, A) `size_of == 0` for both the
`PhantomData<*const ()>` and `_private: ()` forms; the token-carrying and token-free functions
folded (`w_with_token = w_no_token`). → ZERO-COST at the ABI. **RE-VERIFIED 2026-09-03 — HOLDS.** Alias `w_with_token = w_no_token` at codegen-units 16
AND 1 (97 instructions, 45 AVX2 vector ops each — the token does not disturb vectorisation);
across a forced unit split, 0 diff, 97 = 97. EV-10's release gate recompiled beside it:
`size_of::<TokenDbg>() == 8` with `debug_assertions` off.

**EV-10 — `#[cfg(debug_assertions)]` field inside a type.** (L) Release-profile gate
`size_of::<TokenDbg>() == size_of::<usize>() == 8`: the `ThreadId` tripwire is absent from the
type, not merely unused. Compilation-conditional ⇒ guarantee. → ZERO-COST in release.

**EV-11 — sealed trait + `Marker` type parameter vs plain trait.** (A) `sl_sealed = sl_plain`.
→ COMPILE-TIME ONLY. **RE-VERIFIED 2026-09-03 — HOLDS, and the behaviour half now has cross-crate evidence.**
Alias `sl_sealed = sl_plain` at codegen-units 16 AND 1 (70 instructions, 23 AVX2 vector ops, 0
calls); across a forced unit split, 0 diff, 70 = 70. Re-run as a genuine two-crate probe (rlib +
downstream crate, both at the shipped flags): a downstream `impl SealedOp<MarkerA> for Mine` is
`error[E0277]: the trait bound Mine: sealed_dep::seal::Sealed is not satisfied`, while the
plain-trait impl in the same file compiles — the seal is the whole difference and costs nothing.

**EV-12 — `impl Trait` in argument position vs named generic.** (A) `i_call_generic = i_call_apit`.
→ ZERO-COST (documented sugar); the turbofish/semver hazard is real but not a runtime cost.

**EV-13 — return-position `impl Iterator` vs `Box<dyn Iterator>`.** ~~(A, T) Boxed: 69
instructions including `callq ___rust_no_alloc_shim_is_unstable_v2` (a real allocation). RPIT:
44, no allocator call. Timing 0.083–0.086 ns/elem RPIT vs 0.154 boxed = **1.8×**. → RPIT
ZERO-COST; `Box<dyn Iterator>` COSTS.~~ **HOLDS-NARROWER 2026-09-03 — the per-element cost
exists only under two conditions this row never stated, and EV-60's 2.8× is absorbed here.**
(1) DEVIRTUALISATION (EV-124). Written the way this row and EV-60 describe theirs — ONE concrete
iterator type behind the box — LLVM devirtualises it at the shipped profile: `sum_boxed`'s loop is
`vaddss (%rcx),%xmm6,%xmm6; addq $64,%rcx; cmpq; jne`, byte-identical to the RPIT loop, with NO
indirect call anywhere; the survivors are one `__rust_alloc` in the constructor and one
`___rust_dealloc`. The measured "cost" of that shape is one malloc+free per CALL — 0.27 ns/elem at
n = 256 and 0.017 at n = 4096 — a per-call cost impersonating a per-element one at small n. A
genuine `callq *%rbx` in the loop appears only when the box may hold either of TWO concrete types
behind an opaque flag, or is minted and consumed across an `#[inline(never)]` seam (the real API
shape). (2) SCALE. Vtable genuinely dynamic, 64-byte column, three separated runs: L1 (256 rows)
direct 0.615 vs boxed 1.967 [1.409..2.340] ns/elem = 3.20×; L2 (4096 rows) 0.843 vs 2.020 = 2.40×;
64 MiB 14.74 vs 15.19 = 1.03× — the indirect call is entirely hidden behind memory latency. Across
an opaque `#[inline(never)]` seam on the engine's real 12 B + 12 B query pair, six paired runs:
throughput-bound u32 fold 1.59× at 1024 rows (12 KiB) / 1.04× at 65 536 / 0.96× at 1 048 576;
latency-bound f32 reduce (the shape most accumulating query bodies have) 1.17× / 1.05× / 0.89× —
the serial FP-add chain hides the whole call. Asm across the seam: `consume_box_iter_u32` 122
instructions / 51 vector ops / 1 call at cu=16 (131 / 38 at cu=1) against `consume_impl_iter_u32`
114 / 51 / 0 — the same 51 vector ops. So EV-60's 2.8× reproduces only in the dynamic-vtable case
at column sizes up to L2, and this row's 1.8× only at the L1-resident, throughput-bound corner. →
RPIT ZERO-COST; `Box<dyn Iterator>` COSTS an allocation per construction BY DEFINITION and 1.4–3.2×
per element ONLY while the vtable is genuinely dynamic AND the column is L1/L2-resident — parity
beyond the LLC and on a latency-bound body. ERG-12 stays a MUST on the allocation and the hot-path
ban, not on a magnitude.

**EV-14 — dispatch: `#[repr(u8)]` enum tag vs `&dyn` vs fn pointer, TWO loop shapes.**
~~(A, T) Asm: enum tag = zero `call` instructions, loop-invariant `match` hoisted, body vectorised
(`paddq/pxor/movdqu`); `&dyn` = `callq *%r15`; fn ptr = `callq *%rbx`. **Latency-bound loop**
(serial dependency chain): dyn 1.96, tag 1.96–2.12, fnptr 2.01–2.06 ns — NO measurable difference.
**Throughput-bound loop**: tag 0.233 ns/elem = monomorphic 0.233–0.236; dyn 1.163–1.172; fnptr
1.175–1.253 — **5×**. → enum tag ZERO-COST vs monomorphic; `dyn`/fn-ptr COST 5× on throughput
loops and are invisible on latency-bound ones.~~ **HOLDS-NARROWER 2026-09-03 — the throughput
cost is FAR LARGER than 5× where the engine iterates and exactly ZERO beyond the LLC, and the
latency exemption is REFUTED for short chains.** ASM (16 and 1 units): `thr_tag` 123 instructions /
32 vector ops / 34 `ymm` operand lines and no call — the `match` unswitched out and the body
vectorised; `thr_dyn` 40 with `callq *%r12` INSIDE the loop and ZERO vector ops; `thr_fnptr` 36 /
indirect call / 0; `thr_mono` 46 / 8 / 9. On the tree's own 12 B + 12 B query pair
(`query_iter.rs`'s `QBenchPos` / `QBenchVel`): `a_direct` 80 / 42 vector ops / 0 indirect calls;
`a_dyn` 46 / 4 / 1; `a_fnptr` 42 / 4 / 1 — the codegen delta is the DELETED VECTORISATION, and a
12-byte element (eight rows per `ymm`) is why the ratio is larger than on EV-60's 64-byte fixture.
TIME, throughput-bound u64 stream, four separated runs: L1 (16 KiB) mono 0.087 / tag 0.087 (0.99× —
the identity holds) / dyn 2.213 [1.694..2.312] = 25.4× / fnptr 1.722 = 19.7×; L2 (256 KiB) 9.3× /
8.2×; 64 MiB mono 8.364 / dyn 7.483 = 1.00× / fnptr 1.04× — at a DRAM-bound stream the vtable call
is free. On the 12 B + 12 B pair, six paired runs: dyn/direct **10.95×** at 1024 rows (24 KiB, the
`par_iter` chunk floor), **7.24×** at 65 536 rows (a 96 KiB per-worker chunk), 1.14× at 1 048 576
(24 MiB, beyond LLC); fnptr 11.40× / 6.47× / 1.19×; direction 18 of 18. LATENCY-bound (serial
`v = op.apply(v ^ x)`, a ≈2-cycle chain): tag 0.576 [0.557..0.581] vs dyn 1.662 [1.414..1.783] =
2.89× at L1, 2.79× at L2, 1.33× beyond LLC — the 2026-08 exemption held because THAT chain was long
enough to hide a call; it is a property of the chain LENGTH, not of the loop shape. → enum tag =
monomorphic (holds); `dyn` / fn-ptr COST **7–11× on the engine's own per-worker chunk sizes**
(12–256 KiB, which never leave L2 — EV-125) and nothing on a stream past the LLC that no `par_iter`
chunk reaches. State the loop shape AND the working set with every dispatch claim. ERG-14 is
STRENGTHENED, not weakened.

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

**EV-21 — `iter().chain()` vs two sequential loops.** ~~(A, T) `chain` is SMALLER (68 vs 87
instructions) and SLOWER: 0.063–0.072 vs 0.035–0.042 ns/elem — **≈1.7×**. The one case where
reading the asm alone gives the wrong verdict. → COSTS. QUALIFIED (EV-60): 3–8 % at column scale;
the direction reproduces in every run, the 1.7× magnitude is L1-resident scale.~~
**HOLDS-NARROWER 2026-09-03 — the ASM half holds and is the larger half; the TIME half holds only
at L1 on a wide component, and EV-60's 3–8 % does not reproduce on `f32`.** ASM (16 and 1 units):
`chain_form` 21 instructions / 3 vector ops / ZERO `ymm` operands — scalar `vaddss`, not
vectorised; `two_loops` 73 / 21 vector ops. Same picture on the 64-byte column: `chain_col` 23 / 3
against `two_loops_col` 73 / 21. `chain`'s two-state iterator defeats vectorisation of the
reduction — reproducible, structural, and what the rule should cite. TIME, four separated runs.
64-byte column: L1 (256 rows = 16 KiB, below `MIN_ARCHETYPE_FOR_PARALLEL`) chain 0.879
[0.734..0.899] vs two loops 0.694 [0.603..0.732] ns/elem = 1.27×, ranges DISJOINT; L2 (4096 rows)
1.003 vs 0.973 = 1.03×, inside the 6 % band; 64 MiB 23.41 vs 22.45 = 1.04×, far inside the 25 %
band. `f32` halves: L1 1.00×; L2 0.97× (chain FASTER); 64 MiB 1.12×. → a reproducible 27 % on an
L1-resident 64-byte column, nothing distinguishable at any column scale the engine allocates. The
MUST-NOT in ERG-35 and the Part-A refusal REF-14 rested on the 1.7×; both are demoted to a
preference that rests on the codegen fact and on legibility.

**EV-22 — `split_at_mut` vs hand-rolled raw-pointer disjointness.** ~~(A, T)
`split_at_mut`: 53 instr, 2 `addps` + 1 `addss` + 6 `movups` (vectorised). Raw pointers off one
base: 77 instr, 2 `addps` + 5 `addss` + 10 `movss` — scalar fallback, LLVM cannot prove the two
derefs disjoint. 0.140 vs 0.175 ns/elem — the unsafe version is **25 % slower**. → the safe API
wins. SUPERSEDED (2026-09-02, EV-60, EV-70): on rustc 1.97.1 at the shipped profile the raw form
vectorises too and times identically; ERG-24 now rests on review budget, not speed.~~ **REFUTED
2026-09-03, in two different ways at two shapes — and the 2026-09-02 "at worst equal" is
withdrawn with it.** `f32`: EV-70's parity HOLDS — `split_slice` 60 instructions / 15 vector ops /
12 `ymm` lines / 1 panic call, `split_raw` 62 / 18 / 12 / 0, BOTH `ymm`-vectorised at 16 and 1
units; and the timing SIGN FLIPS between two binaries built from the same source at the same
profile (raw 1.5× faster at L1 in one binary, slice 1.4× faster in the other; L2 and 64 MiB inside
the bands) — code placement, EV-49's rule, so `f32` is parity and no speed claim exists in either
direction. **64-byte column — the shape ERG-24 governs — a NEW and OPPOSITE asymmetry neither
EV-60 nor EV-70 recorded:** `split_slice_col` is 23 instructions / 3 vector ops with a
PER-ITERATION bounds decrement (`subq $1, %r10; jb <panic>`) and NO unrolling; `split_raw_col` is
43 / 15, 4× unrolled, zero panic sites — the SAFE spelling is the one that fails to unroll; equal at
16 and 1 units. Time: the raw form faster in 6 runs of 6 at L1 — 0.877 [0.603..0.941] vs 0.377
[0.306..0.559] = 2.33× in one binary, 0.445 vs 0.313 = 1.42× in the other; at L2 the two binaries
disagree (1.06× slice-favouring / 1.21× raw-favouring); at 64 MiB 1.12× / 1.08×, inside the band. →
`f32`: parity. 64-byte column: the raw form is 1.4–2.3× faster while the column is L1-resident and
indistinguishable elsewhere. ERG-24's "never slower / at worst equal" is WITHDRAWN; the rule rests
on review budget ALONE, and its measured-at-site escape (case (a)) is the path for an L1-resident
wide column (ERG-24, ERG-07, REF-19).

**EV-23 — `Index<Id>` vs `get_unchecked` on the hot path.** ~~(A, T) `Index` keeps
`cmpq/jbe` per element plus a cold `panic_bounds_check` tail, ≈7 instr/elem; unchecked unrolls 8×
to ≈2.3. 0.514–0.571 vs 0.359–0.421 ns/elem. → `Index` COSTS on a proven-in-range path. QUALIFIED
(EV-60): the asm delta is real, the time delta did not reproduce on the shapes this engine
iterates.~~ **HOLDS-NARROWER 2026-09-03.** ASM HOLDS on the gathered 64-byte column (the
`BodyEffective` shape, golden-ratio stride): `col_gather_checked` 23 instructions / 3 vector ops /
one `panic_bounds_check` call; `col_gather_unchecked` 57 / 12 / none — the checked form does not
vectorise the gather, the unchecked one does; identical at 16 and 1 units. TIME: a SMALL cost at L1
ONLY — EV-60's "did not reproduce" is right for L2 and beyond and slightly too strong at L1.
Seven runs across two binaries: L1 (256 rows = 16 KiB) checked 0.777–0.819 vs unchecked
0.625–0.675 ns/elem = 1.19–1.24×, ranges disjoint in both binaries; L2 (4096 rows) 1.03–1.04×,
inside the 6 % band; 64 MiB checked 74.4 [32.6..102.9] vs unchecked 39.0 [36.7..114.2] — spans
overlapping almost completely, UNUSABLE, no claim either way. The 1.5× headline is gone; what is
left is ≈20 % at a working set four orders of magnitude below `POOL_MIN_ROWS × 64 B`. → `Index` on
a proven path costs a codegen delta and ≈20 % at L1 only; REF-15 stays a PREFERENCE and is not an
`unsafe` licence.

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
runs. → COSTS one malloc + one free per construction. Resolves EV-29. **RE-VERIFIED 2026-09-03 (C, T) — HOLDS, and the row now carries the TIME number EV-29
could not get.** The delta is EXACTLY +1 alloc and +1 dealloc per construction (3 + 1 against
2 + 0 in a probe that allocates the overriding `String` at the call site in BOTH arms — the
absolutes differ from the 2 + 1 / 1 + 0 above by that one shared allocation; the delta, which is
the claim, matches). Time: FRU 132.5 [120.0..134.2] ns/construction vs literal 89.7 [81.8..97.3] =
**1.48×, ≈40 ns**, two clean process runs at 1 024 / 16 384 / 262 144 constructions, flat across
batch size (a per-call cost, no cache axis). ⚠️ A third run (18:52, load 70 %, 2 concurrent
`rustc`) read FRU 273–315 ns — a 2.4× offset on ONE arm that neither later run reproduces — and is
DISCARDED and named. ≈40 ns is one malloc+free on this allocator, which is what the count
predicts: the two instruments agree (ERG-31 clause 1, REF-13).

**EV-46 — `collect()` / `extend()` with an exact `size_hint` vs the default `(0, None)`.** (C, T)
`collect` from an iterator with the default hint: n = 64 → 1 alloc + **4 realloc**; n = 4096 → 1
alloc + **10 realloc**. With an exact `size_hint` + `ExactSizeIterator`: 1 + 0 at both sizes.
Timing at 4096: 0.66–0.72 vs 0.46 ns/elem (**≈1.5×**). `extend` into a `Vec::with_capacity(n)`:
1 + 0 regardless of the hint. → an exact hint is load-bearing for `collect`; irrelevant once the
caller reserved. **RE-VERIFIED 2026-09-03 — HOLDS; the TIME magnitude above is UNDER-stated.** Allocation
counts reproduce to the realloc: default hint 1 + 10 at n = 4 096, 1 + 14 at 65 536, 1 + 22 at
16 777 216; exact hint 1 + 0 at all three; deterministic across three process runs. Time, clean
medians (run 1 was contaminated on the `exact` arm at L2 / 64 MiB — 1.029 and 9.038 ns against
0.10–0.12 and 2.2–2.8 in the two clean runs — and is named): 16 KiB no-hint 0.959 [0.919..1.080]
vs exact 0.096 [0.090..0.098] ns/elem = 10.0×; 256 KiB 3.502 vs 0.110 = ≈32×; 64 MiB 8.496 vs
2.821 = 3.0×. The gap is wider than the ≈1.5× above because the exact-hint path from a slice is a
`memcpy` specialisation while the hintless path is a push loop that reallocs 10–22 times — a
shape difference from the 2026-08 probe, stated rather than smoothed. Under-stated, never
over-stated (ERG-36, REF-17).

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
ops). ~~→ COSTS per element; hoisting restores the plain loop.~~ → still COSTS per element, and
**the magnitude (RE-VERIFIED 2026-09-03) is four to ten times what EV-60's 1.6× said at L1/L2 —
and ZERO at column scale.** ASM holds exactly as written, and the mechanism is vectorisation:
`once_per_elem` 27 instructions with ZERO vector ops (the state load, the test and a cold
`expect_failed` branch inside the loop); `once_hoisted` 58 / 9 vector ops / 9 `ymm` lines;
`once_param` (the plain-parameter floor) 50 / 9 / 9 — the hoisted form IS the vectorised plain
loop; identical at codegen-units 16 and 1. TIME, u32 in/out pair, seven runs across two binaries:
L1 (16 KiB) per-element 0.559–0.647 vs hoisted 0.039–0.044 ns/elem = **13–16×**; L2 (256 KiB)
0.610–0.628 vs 0.082–0.104 = **6.0–7.4×**; 64 MiB 3.34–3.57 vs 2.97–3.57 = 1.00–1.12×, inside the
25 % band (the [..2.215] / [..2.993] upper bounds are the 100 %-load runs; direction 7 of 7). EV-60's
1.6× was a serious understatement wherever the stream is cache-resident and an overstatement once
it is not — the mechanism is scalar-versus-`ymm`, which is why it vanishes on a memory-bound
stream (REF-28).

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
`codegen-units = 1`, `-C target-feature=+avx2` — ⚠️ an ad-hoc feature list, not the `x86-64-v3`
level the tree ships); EV-60 is the runtime-cost guard's probe (`D:/tmp/guardprobe`, gnu 1.97.1,
`codegen-units = 16`, SSE2 — it predates `cced895a` — `panic = unwind`, timings best-of-7 × 2000
reps at ONE point, n = 4096 over 64-byte `#[repr(C, align(16))]` components = 256 KiB, every
figure reproduced across three process runs); EV-61..EV-70 are the editor's lab (`D:/tmp/editlab`,
gnu 1.97.1, edition 2024, built at `codegen-units = 1` for ICF and re-built at "the shipped 16" —
⚠️ **the flag of that re-build is not recorded, and a bare `--emit=asm` forces one unit
(EV-122)**; EV-61, EV-62 and EV-69's pair were re-taken on 2026-09-03 at an EXPLICIT 16 with a
forced unit split and hold; EV-63 … EV-68 and EV-70's 64-byte sentence carry their "held at 16"
unverified). Nothing under `crates/` was built or linked. Rows EV-02, EV-21, EV-22 and EV-23 above
USED to carry a SUPERSEDED / QUALIFIED note pointing here; since 2026-09-03 each carries its
current verdict IN PLACE with the old one struck through, and EV-60 below is annotated instead of
being the destination of a pointer. EV-64 and EV-67 carry a SCOPED / NOT ADOPTED note pointing at
the second revision's rows (EV-71 … EV-73) below.

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
compares; ~~`get` over the typestate vs the flag IDENTICAL (41)~~ **CORRECTED 2026-09-03: at the
shipped profile typestate `get` is 6 instructions and flag `get` is 8, the diff exactly the
deleted `cmpb $1; jne` — SHORTER, not identical (EV-07)**; `Column` read over `Option<NonNull>` vs
raw IDENTICAL (24; re-taken 20 = 20, 0 diff, at 16 and 1 — EV-05); `layout_checked(RegisteredId)`
with `& (N-1)` vs `get_unchecked` IDENTICAL (2); `mk_r(SdfOp)` vs `mk_c(u32)` IDENTICAL (3;
re-taken 3 = 3, 0 diff); `combine` as `match` vs `if op ==` chains same opcode multiset, enum one
byte shorter; `Option<IslandId>` guard loop one fewer prologue instruction; ~~**`Option<LiveEntity>`
lookup 25 vs 23 instructions** (layout-free, not codegen-free — LLVM merges the two `u32` payload
words into one `u64` and shifts)~~ **CORRECTED 2026-09-03: `live_lookup_c` (raw `*mut` +
`is_null`) and `live_lookup_r` (`match` on the niche) are 18 = 18 instructions with 0 normalised
diff lines at codegen-units 16 and 1, identical down to `shlq $4; cmpq $0,(%rcx,%r8); je` — the
tagged-union conversion is codegen-FREE, and ERG-04 no longer cites 25-vs-23 as its
measure-per-site example.** **RE-VERIFIED 2026-09-03 at the shipped profile:** all six aliases
reproduce at an EXPLICIT codegen-units 16 and at 1; with every pair forced into DIFFERENT units at
16 (13 alias lines at cu=1 collapse to 3), every pair is 0 normalised-diff lines — the identities
are real and the missing alias is partitioning; the whole layout table recompiled green
(`Option<LiveEntity>` 16 with `offset_of!(unit_index) == 8`, and a runtime binary printed
`zeroed().is_none() = true`). Timing: ~~a 2^20-entry sparse array probed 2^20 times at
golden-ratio stride — `Option<usize>` 27.20 ms, `Option<NonZeroU32>` 10.67 ms (2.55×, working set
16 MiB → 4 MiB)~~ **RE-SCALED 2026-09-03 in three independent labs: the DIRECTION holds at every
scale; the 2.55× is one point, and the MECHANISM named ("working set") is not the one measured.**
Identity lens (7 runs, 3–8 concurrent `rustc` throughout): 2^10 entries (16 KiB / 4 KiB, BOTH
L1-resident) 3.21× [3.12..3.23]; 2^14 (256 KiB / 64 KiB, both L2) 3.41× [3.08..3.80]; 2^22
(64 MiB / 16 MiB, neither resident) 1.30× [1.23..1.45]; **2^20 — this row's own scale, exactly the
16 MiB LLC boundary — UNMEASURABLE on this box**, the niche arm swinging 1.762–46.746 ms (26×)
with whatever the concurrent builds were doing to the shared L3, ratio 1.66×–31.6×, nothing
quotable. Timing lens (4 runs, 2^20 probes): 5.4× at 1 024 entries, 5.5× at 16 384, 1.36× at
4 194 304. Engine-shape lens (6 runs, the tree's actual key domains — EV-125): 512 keys (the
`ComponentId` domain, `MAX_COMPONENTS`, 8 vs 2 KiB) 1.0–2.1× SPLIT BY BINARY (0.93–1.11 on one
build, 1.99–2.23 on the next — placement, EV-49); 8 192 keys 0.9–2.1× the same way; 131 072 keys
(2 MiB vs 512 KiB, both L3) 1.89×; 1 048 576 keys (16 vs 4 MiB, straddling the LLC) **7.25× on
four uncontended passes and 1.00× / 0.97× on two passes where nine to twelve `rustc` processes
had evicted the L3 for BOTH arms** — a real regime, reported, not noise hidden. The asm says why
the largest, most stable gap is where both tables are RESIDENT and a working-set argument cannot
apply: the niche loop is 7 instructions with ZERO branches per probe (`None` is the zero word,
`get()` is the identity) while the `Option<usize>` loop keeps `shlq $4; cmpl $1,(%rcx,%rsi); jne`
plus a dependent payload load valid only on the `Some` arm — LLVM cannot if-convert it and a random
tag mispredicts on ≈1/3 of probes; forced branchful on both sides the niche still wins 2.7–3.4×.
→ the sentinel, newtype, typestate and `Option<NonNull>` conversions are ZERO-COST (re-verified);
the tagged-union conversion is codegen-free (corrected); `SparseMap`'s `Vec<Option<usize>>` COSTS
runtime and the fix is free — ERG-04's direction is safe, and its number is **"3–5× while both
tables are cache-resident; 1.3× once both are beyond the LLC; ≈7× only where the wide table crosses
the LLC and the narrow one does not, which on this tree is the two entity-keyed maps at ≈10^6 keys
(`clone::map`, `observers::entity_store`) and nothing else; 1.0–2.1× from one deleted load at the
dominant key domains (`ComponentId < 512`, `ArchetypeId`)"**. "2.55×" and "16 MiB → 4 MiB" are not
carried; the 2^20 point needs a quiet machine.

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
SMALLER, refuting the previous ERG-16 reason. **ANNOTATED 2026-09-03.** This row's per-claim verdicts are ABSORBED into the rows they
qualified — EV-02, EV-13, EV-14, EV-21, EV-22, EV-23 and EV-56 each now carry the current verdict
in place with this row's number beside the 2026-09-03 ones — so no reader follows a pointer here
for a verdict. Its fixture is re-scoped (EV-125): n = 4096 × 64 B = 256 KiB is a good L2 point and
a poor whole-column point. The engine's most-iterated components are 12–48 B (`boyko_ecs`'s own
query bench iterates 12 B + 12 B; `Transform` 40, `GlobalTransform` 48, `UiText` 12); 64 B is the
TOP of the range and the physics shape (`BodyEffective`, align 4, not 16); a query fetch walks two
or three columns plus a 4-byte per-row `Tick`; and there was no L1 point and no beyond-LLC point
at all. Two of its ratios change VERDICT as a function of scale alone (dispatch 10.95× / 7.24× /
1.14×; sparse width 1.0–2.1× / 1.9× / 7.25×), and one changes with element width (dispatch 2.5–3×
at 64 B against 10.95× at 12 B, because a narrow element is what lets the direct loop pack eight
rows per `ymm`). Its two headline REFUTATIONS (`get_unchecked`, `split_at_mut` on `f32`)
reproduce at every engine shape; its `chain` 3–8 % does not reproduce on `f32` (EV-21); its
`split_at_mut` parity does NOT extend to the 64-byte column (EV-22). Taken at SSE2, before
`cced895a`, so its absolute figures do not transfer.

**EV-61 — `if const { T::FLAG }` vs `if T::FLAG` vs no arm.** (A, B) `ic_plain_table`,
`ic_const_table` and `ic_none` (a hand-written loop with no arm) emitted as ONE symbol
(`ic_plain_table = ic_const_table`, `ic_none = ic_const_table`) — at `codegen-units = 1` AND at
the shipped 16; 39 instructions, `paddd`-vectorised. The `= true` instantiations: 35 instructions,
0 normalised-diff lines between the two spellings. `if const { self.flag }` → `error: attempt to
use a non-constant value in a constant`. → ZERO-COST; the `const { }` form additionally refuses
the runtime-state refactor at compile time (ERG-31). **RE-VERIFIED 2026-09-03 — HOLDS in full, counts restated at the shipped ISA.** At
`x86-64-v3`, codegen-units 16 AND 1: `kernel_plain::<NoMask>` and `kernel_const::<NoMask>` are ONE
SYMBOL (explicit alias), and `ic_none` is 0 normalised-diff lines against it — 49 instructions, 13
AVX2 vector ops (`vpaddd` / `vmovdqu` over `ymm`), ZERO calls; the dead arm's mask load AND the
mask slice's bounds check are both gone. The `= true` pair is 51 = 51, 15 vector ops, 0 diff. The
counts are 49/51 rather than 39/35 because the row was taken at SSE2; the identity, which is what
the row licenses, is unchanged. `if const { self.flag }` re-run: `error: attempt to use a
non-constant value in a constant … help: try using \`Self\``.

**EV-62 — `Drop` guard vs manual epilogue, `panic = unwind`.** (A) Body an opaque `fn()` pointer
(may unwind): ~~`run_manual` 20 instructions, `run_guard` 27 — the normal path is IDENTICAL (the
same 20), the delta is exactly the cleanup landing pad: `movq; callq drop_glue::<Guard>; movq;
callq _Unwind_Resume; ud2; callq panic_in_cleanup; ud2` plus a `GCC_except_table` entry.~~
**HOLDS-NARROWER 2026-09-03 — the structure reproduces exactly; the 7-instruction figure is
CONDITIONAL on something the row did not state.** At the shipped profile with an INLINABLE
`Drop`: `run_manual` 9 instructions, `run_guard` 13, the normal path byte-identical (`pushq %rsi;
subq $32,%rsp; movq %rcx,%rsi; incl (%rcx); callq *%rdx; decl (%rsi); …; retq` in both), and the
delta is exactly and only the landing pad — but it is FOUR instructions (`decl (%rsi); movq
%rax,%rcx; callq _Unwind_Resume; ud2`) plus `.seh_handler rust_eh_personality, @unwind, @except`
and a `GCC_except_table` entry: no `drop_glue` call, no `panic_in_cleanup` block. The seven WITH
`drop_glue` reproduce only with `#[inline(never)] fn drop` — 17 instructions, FOUR calls — and
THAT form puts a `callq <Guard as Drop>::drop` ON THE NORMAL PATH and grows the frame from `subq
$32` to `subq $48`. Body a plain arithmetic statement (cannot unwind): `run_manual_nounwind =
run_guard_nounwind` (ICF alias) at 1 and 16 units, re-verified 2026-09-03 at an explicit 16 and
across a forced unit split (0 diff). → ZERO-COST on the normal path EXACTLY WHEN the guard's
`Drop` inlines — which a one-line depth / cursor guard's does; the landing pad IS the thing bought
(ERG-43, whose cost sentence now carries the condition).

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
EV-60: parity. → supersedes EV-22's "scalar fallback"; ~~the slice form is at worst equal (ERG-24,
REF-19)~~ — WITHDRAWN 2026-09-03 for the 64-byte column, see the note that follows. **RE-VERIFIED 2026-09-03 for `f32` (through EV-22): HOLDS** — both forms `ymm`-vectorised at
`x86-64-v3` (60 / 15 vector ops against 62 / 18), parity in time (the sign flips with placement
between two binaries). ⚠️ **"The slice form is at worst equal" does NOT extend to the 64-byte
column:** there the slice form fails to unroll (23 instructions, a per-iteration bounds decrement)
against the raw form's 43 / 4× unrolled, and is 1.4–2.3× slower while L1-resident — EV-22. "At
worst equal" is withdrawn from ERG-24 and ERG-07.

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

## Rows added by the second technique sweep (2026-09-02) — the source-cut survivors, compiled

**Where.** `D:/tmp/erglab4` (outside the repository), `rust-toolchain.toml` pinning
`stable-x86_64-pc-windows-gnu`, `rustc 1.97.1 (8bab26f4f 2026-07-14)` confirmed, edition 2024,
`opt-level = 3`, **`codegen-units = 16` — the SHIPPED profile** — no LTO, `panic = unwind`, and
`.cargo/config.toml` mirroring the tree's `-C target-cpu=x86-64-v3`. Every identity was
**re-checked at `codegen-units = 1`** and the unit count is named per row where it differed (it
never did). Unlike `editlab3`, this lab DID emit identical-code-folding alias lines — as
`sym_a = sym_b`, not `.set` — so where a row says "ICF alias" that is the strongest identity
evidence the method section describes; where it says "N diff lines" the pair did not fold and the
label-normalised diff is the evidence. Behaviour probes are single-file `rustc --emit=metadata`
compilations (no codegen) plus five two-crate probes against the lab's own rlib. A timing binary
in the same crate carries the counting `#[global_allocator]`.

⚠️ **EVERY TIMING IN EV-77 … EV-95 WAS TAKEN UNDER LOAD** — a game plus three concurrent build
workflows were running on this box throughout. Each figure is best-of-7 reported across **three
independent process runs**, and all three are printed so the spread is visible. Directions
reproduced in every case; magnitudes did not, and no absolute ns figure here should be quoted
without the caveat. Nothing under `crates/` was built, modified or linked into any probe.

**EV-77 — the featured island: `unsafe` around a SAFE `#[target_feature]` kernel vs the caller
carrying the attribute.** (A, B) `caller_plain` (a plain `fn` with one `unsafe` block spanning
loads, the kernel call and the store) and `caller_boundary` (a `#[target_feature(enable =
"avx2")]` island whose kernel call carries NO `unsafe`, reached through one boundary `unsafe`):
**14 instructions each, 0 normalised-diff lines** at codegen-units 16 and 1, and the kernel symbol
appears NOWHERE in either `.s` — a same-feature callee inlines. Behaviour: a plain `fn` calling a
safe `#[target_feature(enable = "avx2")]` fn is `error[E0133]: call to function ... with
#[target_feature] is unsafe and requires unsafe block` — **under `-C target-cpu=x86-64-v3`**, so
neither the tree's global baseline nor `-C target-feature=+avx2` discharges it; the CALLER must
carry the attribute. Interaction with ERG-15, also probed: a `#[target_feature]` fn cannot be
coerced to a safe `fn(..)` pointer (`E0308`) and cannot be passed to an `Fn` bound (`E0277`,
"expected a Fn closure, found #[target_features] fn") — the featured half of a shell/inner split
stays concrete. → ZERO-COST; the rule deletes obligations rather than documenting them (ERG-46).

**EV-78 — the perfect-derive trap: `#[derive(Clone, Copy)]` vs hand impls on a generic tag
struct over `PhantomData<fn() -> R>`.** (A, B, L) **ICF aliases `dup_hand = dup_derived` and
`pair_hand = pair_derived`** at codegen-units 1 AND 16 — the copy body and the tuple-state
destructure are byte-identical. Layout: both forms 4 bytes over a NON-`Copy` `R`. Behaviour:
`assert_copy::<DerivedState<NotCopy>>()` is `error[E0277]: the trait bound NotCopy: Copy is not
satisfied` — the derive emits `impl<R: Clone> Clone` / `impl<R: Copy> Copy`, so the state is
`Copy` only when the resource is; the hand impls compile. → ZERO-COST; the hand impls are strictly
stronger (ERG-47).

**EV-79 — index width: `usize` ids and a 16-byte `Entity` vs `u32` ids and an 8-byte one.**
(L, A, T) Layout: `EntityWide { EntityId(usize), u32 }` = 16 (4 bytes of pure padding);
`Option<EntityWide>` = **24** — the lab's own gate was written as 16 and failed with `E0080`,
because rustc does NOT place the tag in the tail padding. `EntityNarrow { u32 slot, NonZeroU32
generation }` under `#[repr(C, align(8))]` = 8, align 8; `Option<EntityNarrow>` = **8** (the
generation niche is free); `Option<ComponentId(usize)>` = 16 vs `Option<ComponentId(u32)>` = 8;
`[EntityWide; 4]` = 64 bytes = `[EntityNarrow; 8]`. Asm: `Ord` over `to_bits()` is **4
instructions** (`movq; cmpq; seta; sbbb`) against the derived lexicographic `Ord`'s **12 with a
branch**; `eq` 8 = 8; `is_alive` over a slot table 7 vs 8 (one `movl` zero-extend). Timing UNDER
LOAD, a 2^20-entry table probed 2^20 times at golden-ratio stride, best-of-7 × 3 process runs:
13.29 / 12.34 / 15.57 ms wide against 6.35 / 8.30 / 9.48 ms narrow = **2.09× / 1.49× / 1.64×** —
the same working-set-halving class EV-59 measured at 2.55×. → the width change is ZERO-COST in
codegen and NEGATIVE-cost in memory and time (ERG-02, ERG-04). **ANNOTATED 2026-09-03.** The 2.09× / 1.49× / 1.64× is ONE point (2^20 entries, exactly the
LLC boundary) under load, in the index-width class EV-59 has since re-scaled: 3–5× while both
tables are cache-resident, 1.3× once both are beyond the LLC, ~7× only where the wide table crosses
the LLC and the narrow one does not. Direction holds at every scale; the layout and `Ord` halves
are unaffected. Read as one point, not as the class's constant (ERG-02 clause 5).

**EV-80 — a storage policy as an associated TYPE vs two independent `STORAGE_IS_*` bools.**
(A, B, L) **ICF aliases in BOTH instantiations**, at codegen-units 1 and 16: `loop_bools_table =
loop_assoc_table` and `loop_bools_dense = loop_assoc_dense` — `if const { C::STORAGE_IS_DENSE }`
and `if const { matches!(<C::Storage as StorageClass>::KIND, StorageKind::Dense) }` compile to one
symbol. `size_of` of every marker ZST is 0. Behaviour: `pool_of::<Flagged>` at a
`C: Component<Storage = TableStorage>` bound is `error[E0271]: type mismatch resolving
<Flagged as Component>::Storage == TableStorage`; a downstream `impl StorageClass for MyStorage`
is `error[E0277]: the trait bound MyStorage: seal::Seal is not satisfied`. → ZERO-COST; the
associated type is the only one of the two that can appear in a where-clause, and `(true, true)`
stops being representable (ERG-31 clause 3).

**EV-81 — cache-line PADDING width: 64 (the tree's hand-rolled attribute) vs 128 (crossbeam's
table).** (L, A) `crossbeam_utils::CachePadded<T>` is **align 128, size 128** on
`x86_64-pc-windows-gnu` — confirmed by a `const _` gate and independently by
`-Zprint-type-sizes`. The tree's `boyko_log::lane::LogLane` partition shape modelled as
`#[repr(C, align(64))]`: 192 bytes, `offset_of!(read) == 64` (its own gates); the same three
partitions as `CachePadded` fields: **384 bytes, align 128, `offset_of!(consumer) == 128`** — the
memory cost is exactly 2×, and it is the number a site must state. Accessor asm is unaffected:
`fetch_add(1, Release)` through both is the same 3 instructions (`movl $1; lock xaddl; retq`),
0 diff lines. Census on this checkout: **39 hand-rolled `align(64)` sites in 20 files across 8
crates, ZERO `align(128)`, and `CachePadded` in 7 files** — the two halves of the engine disagree
and the 64-byte code LOOKS padded. → the LAYOUT half is settled; **the RUNTIME half is NOT
measured** — `crates/boyko_log/benches/lane_padding_ablation.rs` compares padded-64 against
unpadded and would need a third arm on an IDLE machine (ERG-01 clause 4).

**EV-82 — `#[derive(bytemuck::Pod)]` as the no-padding proof an ERG-01 size gate cannot give.**
(A, B, L) **ICF alias `bytes_raw = bytes_pod`** at codegen-units 1 and 16 — `bytemuck::bytes_of`
and a hand `slice::from_raw_parts` are one symbol. `Mat4Pod` (`#[repr(C, align(16))]`, 16 `f32`)
accepts the derive and keeps its 64/16 gate. Behaviour: `#[derive(Pod)]` on
`#[repr(C)] { a: u16, b: u32 }` is `error[E0080]: evaluation panicked: derive(Pod) was applied to
a type with padding` — while `assert!(size_of::<Padded>() == 8)` PASSES, which is the whole point:
the size gate cannot see the two padding bytes a device would read uninitialised. → ZERO-COST
(ERG-01 clause 5).

**EV-83 — strict provenance: `ptr as usize` vs `.addr()` / `expose_provenance`.** (A) **Four ICF
aliases at codegen-units 1 and 16**: `seed_cast = seed_addr` (the splitmix64 hash seed),
`align_cast = align_addr` (the SIMD-alignment test), `roundtrip_expose = roundtrip_cast`,
`roundtrip_back_cast = roundtrip_back` (`with_exposed_provenance` against a raw
`addr as *const T`). → ZERO-COST, a pure spelling change. ⚠️ The BENEFIT — that the roundtrip
stops being UB under `-Zmiri-strict-provenance` — was NOT measured: no Miri run was made in this
pass (ERG-22 clause 4, OPEN 1).

**EV-84 — a container typed by its index (`IndexSlice<I, T>`) vs `xs[id.0 as usize]`.** (A, B)
`sum_raw` and `sum_typed` over a 16-byte element: **27 instructions each, 1 normalised-diff line**
— the panic-location constant. Behaviour: indexing an `IndexSlice<ArchetypeId, Archetype>` with a
`ComponentId` is `error[E0308]: mismatched types`. The `Index` body IS `&self.raw[i.index()]`, so
REF-15's refusal (which is about `get_unchecked`) does not bite. → ZERO-COST (ERG-02 clause 5).

**EV-85 — an FFI descriptor that carries the borrow's lifetime, and a typed `pNext` chain.**
(L, A, B) `size_of`, `align_of` and `offset_of!(p_queue_create_infos)` are EQUAL between the bare
`#[repr(C)]` create-info and the `<'a>`-parameterised one with a `PhantomData<&'a ()>` tail — a
lifetime has no representation, so `#[repr(C)]` compatibility survives. `mk_bare` 9 = `mk_typed` 9
instructions, the only difference a reordered store. Behaviour: `let c = mk_ci(&v); drop(v);
submit(&c)` is `error[E0505]: cannot move out of v because it is borrowed`, while the bare form
COMPILES; `push_next` with a struct that lacks the `unsafe trait ExtendsDeviceCreateInfo` marker
is `error[E0277]`. → ZERO-COST (ERG-06 clause 3).

**EV-86 — constructing IN the reserved row (`add_with(|| T)`) vs by value (`add_typed(T)`).**
(A, L) SMALL component (4 bytes): **ICF alias `tiny_with = tiny_value`** — the closure form costs
nothing. LARGE component (`#[repr(C)] [u64; 512]`, 4096 bytes) with the pool method
`#[inline(never)]`: `add_typed_ni` is 24 instructions containing **one `memcpy` call**, and its
caller carries a `___chkstk_ms` stack probe — a frame over 4 KB; `add_with_ni` is 146 instructions
with **zero `memcpy`**, and its caller's frame is `subq $40, %rsp` with no probe. With the method
`#[inline]`, the by-value caller STILL carries the probe (285 instructions) and the closure caller
still does not (300 instructions, 40-byte frame). In the n-push loop the two converge (159 vs 161,
10 diff lines). → ZERO-COST at small `T`; NEGATIVE-cost at large `T` (it deletes a caller frame and
a stack probe), which is EV-08's physics from the constructor side (ERG-08 clause 3).

**EV-87 — one kernel body over a `Lane` trait, instantiated at `__m256` and at `f32`.** (A, B)
AVX2 arm at `-C target-cpu=x86-64-v3`: the generic body inside a `#[target_feature]` wrapper and
the hand `_mm256` transcription are **87 instructions each with IDENTICAL opcode histograms**
(18 `vmaxps`, 9 `vsubps`, 9 `vaddps`, 6 `vmovups`, 6 `vmovaps`, 3 `vmulps`, 3 `vminps`, 3
`vbroadcastss`), **zero `call`, zero `vfmadd`**; the per-kernel pair 30 = 30. Scalar arm:
bit-identical to the hand twin over **10^6 random inputs, 0 mismatches, 3 runs** — but ONLY after
the `impl Lane for f32` was corrected to call `f32::min` / `f32::max`; the lab's first draft wrote
`if self < o { self } else { o }`, whose asm lacks the `vcmpunordps` / `vblendvps` NaN pair the
std call emits. That is the trap this technique carries. Even matched, the generic scalar body is
**37 instructions against the hand twin's 29** — an `xmm6` spill and a 24-byte frame that the hand
form does not have. → ZERO-COST on the AVX2 arm; a small frame cost on the scalar arm; the
`#[inline(always)]` on the generic body is STRUCTURAL (the vector ops must compile inside the
featured wrapper) and still owes ERG-28's measurement line (ERG-15 clause 4).

**EV-88 — a third assertion tier on a cargo feature, and `MaybeDbg<T>` for the debug-only field.**
(L, A) With `--features boyko-assert` in the **release** profile, `from_axis_angle` goes 22 → 35
instructions and gains exactly one panic site; without it, 22 and none. The
`#[cfg(debug_assertions)]`-gated size gate compiles unchanged in both — **the tier changes checks,
not layout**, which is the configuration a bit-determinism campaign wants and which
`debug_assertions` cannot give. `MaybeDbg<u64>` is **0 bytes in release, 8 in debug**, and the host
token's size equals the raw-`cfg`-field form in both profiles; **ICF alias `check_cfg =
check_wrapped`** at codegen-units 1 and 16. → COMPILE-TIME ONLY (the wrapper) plus the checks the
feature explicitly buys (ERG-26 clauses 4 and 5).

**EV-89 — `core::hint::cold_path()` in a branch whose slow arm is already a `#[cold]` fn.** (A)
`push_plain` (today's `if self.len >= self.committed_rows && !self.grow_rows(..)`) and `push_cold`
(the same with `core::hint::cold_path()` in the growth arm): **0 code diff lines** at codegen-units
1 and 16 — the only cu-16 difference is the `@feat.00` / `.text` assembler directives — and the
`callq grow_rows` sits at the same block position in both. `#[cold]` on the callee already carried
the information. → NO EFFECT at the named site; refused as a rule (REF-43). `cold_path` IS stable
on `rustc 1.97.1` (probed), as are `assert_unchecked` and `select_unpredictable`.

**EV-90 — `core::hint::select_unpredictable`.** (A, T) Arithmetic form (`acc += if d < r { hit }
else { miss }`): **ICF alias `select_form = branch_form`** — LLVM already emits a vectorised blend
and the hint buys NOTHING. Two-read form (`if p { a[i] } else { b[i] }`): ~~`branch_read` 343
instructions / 54 branches against `select_read` 140 / 12 with 5 blends. Timing UNDER LOAD, 2^20
elements, best-of-7 × 3 runs — **random 50/50 predicate**: 2.957 / 2.934 / 2.956 ms branch against
0.328 / 0.351 / 0.912 ms select = **9.01× / 8.35× / 3.24× faster**; **sorted predicate**: 0.317 /
0.317 / 0.987 against 0.377 / 0.271 / 0.665 = **0.84× / 1.17× / 1.48×** — the well-predicted case
LOST 19 % in one run and never won reliably.~~ **HOLDS-NARROWER 2026-09-03 — the instruction
counts are CODEGEN-UNIT-DEPENDENT and are not carried; the 8–9× does not exist; a real 2.5× exists
at exactly one point; the "sorted case loses" warning does not reproduce.** ASM at an explicit 16
units: `branch_read` 35 instructions / 3 vector ops / 2 panic calls and `select_read` 99 / 8 / 2 —
the SELECT form is the BIGGER one, the reverse of 343-vs-140; at 1 unit `select_read` is 36 / 3 /
2, equal to the branch form. This is one of only two symbols in the whole lab whose asm differed
between 1 and 16 units, and precisely the one whose row quoted a count. TIME, four separated runs,
u8 predicate + two f32 arrays (9 B/elem), random 50/50: L1 (16.4 KiB) branch 0.836 vs select 0.844
ns/elem = 1.01×, NO WIN; L2 (262 KiB) 2.224 [1.807..2.392] vs 0.871 [0.729..0.919] = **2.55×
WIN**, ranges disjoint; 67 MiB 4.688 vs 4.771 = 1.02×, NO WIN. Sorted predicate: 1.00× / 1.02× /
1.00× — no win and no loss at any scale; the 19 % loss was that run's noise. → the hint buys
something in ONE narrow band (a genuinely unpredictable branch over data that is L2- but not
L1-resident) and nothing on either side; ERG-28 clause 5's requirement to measure BOTH distributions
at the site is correct and now also SCALE-gated. The arithmetic-form alias stands.

**EV-91 — non-short-circuiting `&` in a fold that must vectorise.** (A, T) `eq_andand`
(`eq = eq && (a[k] == b[k])`): **105 instructions, ZERO vector ops, 20 branches — not
vectorised.** `eq_bitand` (`eq &= a[k] == b[k]`): **98 instructions, 36 vector ops, 5
`vpcmpeqb` — vectorised.** Timing UNDER LOAD over two 1 MiB slices, best-of-7 × 3 runs: 0.303 /
0.277 / 0.277 ms against 0.073 / 0.018 / 0.019 ms = **4.16× / 15.14× / 14.35×**. ⚠️ Scope,
measured and stated: a COUNTING loop (`if a && b && c { n += 1 }` against
`n += u32::from(a & b & c)`) vectorises BOTH ways (120 against 100 vector ops, 192 against 171
instructions) — the operator is load-bearing where the predicate feeds an accumulator the NEXT
iteration reads, not universally. → the `&` form is a real change with a real scope (ERG-35
clause 5). **RE-VERIFIED 2026-09-03 — direction HOLDS 4 of 4; the magnitude is LARGER cache-resident
and COLLAPSES beyond the LLC, which the row did not say.** ASM at `x86-64-v3`, 16 and 1 units:
`eq_andand` 74 instructions / ZERO vector ops; `eq_bitand` 86 / 28 vector ops / 21 `ymm` lines
(74/86 rather than 105/98 because the ISA changed; the qualitative fact — one form vectorises, the
other does not — is unchanged and is what the clause cites). TIME, two u8 slices: 16 KiB total
`&&` 0.310 [0.292..0.321] vs `&` 0.0117 [0.0110..0.0120] ns/elem = **26.5×**; 256 KiB 0.319 vs
0.0183 = **17.4×**; 64 MiB 0.604 [0.500..0.666] vs 0.437 [0.284..0.458] = **1.38×**. The 4.16–15.14×
spread above (a 3.6× span across three runs) is explained by exactly this: the three runs were not
at the same effective cache residency under load. Honest scope for ERG-35 clause 5: 17–27× while
the compared buffers are cache-resident, ~1.4× once they are not.

**EV-92 — the gathered-index cohort kernel: OPEN 9, measured at last.** (A, T) A 64-byte
`BodyEffective` column, sixteen rows gathered per 8-wide cohort off `a[lane]` / `b[lane]`, at
`-C target-cpu=x86-64-v3`. Asm: (a) `row_ptr` under a `debug_assert!` — 108 instructions, 0 panic
sites; (b) safe `[]` per lane — 261 instructions, **13 panic sites**; (c) safe `[]` under ONE
HOISTED `assert_unchecked(max_gathered < bodies.len())` — 371 instructions and **13 panic sites
STILL**: LLVM cannot connect the cohort's recorded maximum to `a[lane]`, so the hoisted promise
discharges nothing and grows the body; (d) `assert_unchecked` repeated PER LANE — 109
instructions, 0 panic sites; (e) `get_unchecked` — 107 instructions, 0 panic sites. Timing UNDER
LOAD (256 KB column, 4096 cohorts × 16 rows, best-of-7 × 3 runs, all four forms agreeing
bit-for-bit): row_ptr **0.743 / 1.039 / 0.919 ns/row**; safe `[]` **0.589 / 0.792 / 0.723**;
per-lane promise **0.456 / 0.592 / 0.555**; `get_unchecked` **0.462 / 0.606 / 0.464**. ~~→ in this
shape the SAFE `[]` is **0.76–0.79× of `row_ptr`** — faster, three runs out of three, despite 13
panic sites — and eliding the check buys a further ≈1.3×.~~ **REVERSED 2026-09-03 — two
independent labs, neither reproduces the direction and one reverses it.** (1) A two-gathers-per-lane
READ cohort over a 64-byte column, six runs in two binaries, ratios to the safe form (safe /
`get_unchecked` / `row_ptr`): L1 1.00 / 0.96 / 0.97 and 1.00 / 0.94 / 0.98; L2 1.00 / 1.04 / 0.99
and 1.00 / 0.88 / 0.93; beyond LLC UNUSABLE (per-arm spans 26.9–115.8 ns). All three within 12 % —
parity. That kernel's asm (`cohort_safe` 47 instructions / 5 vector ops / 3 `panic_bounds_check`;
`cohort_unchecked` 66 / 18 / 0; `cohort_rowptr` 69 / 18 / 0) is a DIFFERENT kernel from this row's
(2 gathers per lane, not 16 per cohort) and makes no claim against the 261 / 13 above. (2) The
tree's ACTUAL solve shape — 64-byte `BodyEffective`, two rows gathered per contact off `body_a` /
`body_b`, READ-MODIFY-WRITE of both (the real kernel writes velocities back; this row's kernel only
summed), at the real pyramid geometry (base 141 → 10 011 bodies / 29 751 contacts): `c_safe` 97
instructions / 59 vector ops / 3 panic sites, `c_rowptr` (`ScratchSolveView::row_ptr` verbatim —
`base.add(i)` then `&mut *`) 82 / 59 / 1, `c_unchecked` 73 / 59 / 0 — all three the SAME 59 vector
ops, at 16 and 1 units; the in-tree comment "the slice form could not elide its panic branch" is
confirmed as a codegen fact and confirmed as irrelevant to speed. Time, paired per pass, six runs,
safe / row_ptr: 1 024 bodies / 3 048 contacts (64 KiB, L2) 1.02×; **10 011 bodies / 29 751 contacts
— the 10k pyramid, 625 KiB — 1.05×** (1.08, 1.09, 1.17, 0.98, 1.00, 1.03); 262 144 bodies /
786 432 contacts (16 MiB, at the LLC) 1.10× on the four clean passes (passes 2 and 4 read 2.33 and
5.14 — one arm stalled, the pair did not move together — discarded and named). `get_unchecked` /
`row_ptr`: 1.00× / 0.96× / 1.12×. The safe form is at PARITY to 10 % SLOWER at every scale and
never faster: the sign of 0.76–0.79× is wrong in eighteen of eighteen paired comparisons. WHAT
SURVIVES: the HOISTED `assert_unchecked` is still refuted — REF-44 stands on the asm half above (13
panic sites survive, 261 → 371), which is this row's own shape and was not re-taken — and "the
slice form could not elide its panic branch" is still not by itself a reason to prefer `row_ptr`.
WHAT IS WITHDRAWN: the positive claim that the safe form is FASTER, and the "≈1.3× for eliding the
check". ⚠️ Both re-verification kernels are SCALAR, like this one; the tree's `solve_color_avx2` is
8-wide with ≈25 stack scratch arrays and different aliasing. OPEN 9 is narrowed further and still
not closed — but the direction the site measurement should test first no longer flips: the three
spellings are within ≈10 % of each other, with a small, consistent edge to the raw form at the
real pyramid size.

**EV-93 — the entity free list threaded through the dead slots' own bytes.** (L, C, A)
`enum Slot { Live { NonNull<Archetype>, u32, u32 }, Dead { Option<NonZeroU32>, u32 } }` is
**16 bytes, align 8** — the same as today's `EntityInland`, so the free chain fits in the dead
slot with the `NonNull` niche as the tag. Counting allocator, spawn 2^16 / free all / respawn all:
the side-`Vec` form **3 alloc + 28 realloc + 1 dealloc**; the intrusive form **2 alloc + 14
realloc + 1 dealloc** — identical across three runs. Asm: `is_alive` 14 = 14, 0 diff lines;
`alloc_id` 37 against 56 instructions (the intrusive form's `match`), `free_id` 34 = 34 with 34
diff lines. → the layout claim holds and the despawn-burst reallocations halve; the id arithmetic
is not free, so this is an ARCHITECTURE item with a measurement, not a rule (ERG-04's After,
index OPEN 10).

**EV-94 — `ParamSet`: conflicting parameters made non-simultaneous by `&mut self`.** (A, B)
`via_set` (`set.p0()` … `set.p1()`) 110 instructions against `via_direct` (two separate column
references) 121 — the set form is SMALLER because both columns derive from one base, so the pair
is not a clean identity and no zero-line claim is made. Behaviour: holding `p0`'s item across a
`p1()` call is `error[E0499]: cannot borrow *set as mutable more than once at a time`. → free,
and refused as a GUIDE rule because it is scheduler feature work (REF-45).

**EV-95 — the instruments behind the ledger's own bloat refusals.** `-Zprint-type-sizes` on
`nightly-x86_64-pc-windows-gnu 1.100.0-nightly (8925ea358 2026-08-20)` ran on the lab and ranked
its types by size, independently confirming `CachePadded<*> == 128` (EV-81). ⚠️ `cargo llvm-lines`,
`cargo bloat` and `cargo mutants` are **NOT INSTALLED on this box** and were NOT run: naming them
in the Method section would name an instrument nobody here has used, which is the failure this
ledger exists to prevent. Only `-Zprint-type-sizes` is added to the Method section, with its exact
nightly named as the owner's floating-nightly ruling requires.

**EV-96 — alignment as a TYPE PARAMETER on the type-erased pointer (`RowPtr<'a, A: IsAligned>`).**
(A, L, B) Three pairs, at codegen-units 16 (the SHIPPED profile) and again at 1. Byte-packed push
(`ptr::write_unaligned` off a `*mut u8` against the same through `RowPtr<'_, Unaligned>`):
**73 = 73 instructions, identical opcode histograms, 0 normalised diff lines.** Byte-packed pop:
**60 = 60, 0 diff lines.** The ALIGNED regime (a strided `ComponentPool`-shaped row read against
`RowPtr<'_, Aligned>`): **150 = 150, 0 diff lines.** Layout: `size_of::<RowPtr<'_, Aligned>>() ==
size_of::<*mut u8>() == 8`, align 8, `Option<RowPtr<'_, Aligned>>` still 8, and both markers are
ZSTs (`size_of::<Aligned>() == size_of::<Unaligned>() == 0`). Behaviour: passing a
`RowPtr<'_, Unaligned>` where `RowPtr<'_, Aligned>` is expected is `error[E0308]: mismatched
types`; a downstream `impl IsAligned for MyAlign` is `error[E0277]: the trait bound `MyAlign:
sealed::Sealed` is not satisfied` — the seal holds across the crate boundary.
→ ZERO-COST (ERG-48 shape 1).

**EV-97 — distinctness of an index stream as an empty sealed `unsafe` MARKER TRAIT.** (A, B)
**ICF alias `solve_prose = solve_bound` at codegen-units 16 AND 1** — the loop written through a
bare `unsafe fn row_ptr` with the disjointness obligation in prose, and the same loop behind a
`V: DistinctIndexWrites + RowWrite` bound, are ONE SYMBOL. The 16-row-per-cohort gather (the
`solve_color_avx2` shape): `cohort_prose` **43 = 43** `cohort_bound`, identical histograms,
**0 normalised diff lines**. Behaviour: a type with the row accessor but WITHOUT the marker is
`error[E0277]: the trait bound `Sloppy: DistinctIndexWrites` is not satisfied`; a downstream
`unsafe impl DistinctIndexWrites for Downstream` is `error[E0277]: … `Downstream: sealed::Sealed`
is not satisfied`. In-tree count, no build: `colored.rs` has **50** `// SAFETY:` blocks, of which
**10 restate the coloring / disjointness invariant** — plus the two `unsafe impl Send` /
`unsafe impl Sync`, whose entire justification is that invariant; `row_ptr(` has **84 call sites
across 19 files**. → ZERO-COST (ERG-48 shape 2).

**EV-98 — completion-based ownership: the resource returns through a CALL, not a destructor.**
(A, L, B) A 24-byte device handle. `flow_prose` (`submit_raw` then `wait_fence` then `destroy`
under a prose precondition) and `flow_token` (`submit` then `reclaim(Pending<T>) -> T` then
`destroy`): **14 = 14 instructions with identical opcode histograms**, 6 normalised diff lines,
every one a register-name choice (`%rax` against `%rcx`) — not a clean identity, so no zero-line
claim is made. Layout: `size_of::<Pending<Accel>>() == 32` = the 24-byte payload plus the fence
id. Behaviour: dropping the token is `error: unused `Pending` that must be used` under
`#![deny(warnings)]`. WARNING — **the honest falsifier holds: `core::mem::forget(pending)`
COMPILES SILENTLY**, which is why the payload is `ManuallyDrop` and the device owns the bytes,
i.e. the rule is structural and a `Drop` tripwire would not have bought it. In-tree price, no
build: `destroy_accel` has **4 direct call sites** plus one through the RHI trait, and there are
**118 "fence-waited" / `wait_idle` preconditions across 23 files** of `boyko_rhi_vulkan` and
`boyko_render`. → ZERO-COST (ERG-48 shape 3).

**EV-99 — the linear "must give it back" ticket whose `Drop` is a DEBUG-ONLY tripwire.**
(L, A, B) This is the row that decides the shape, and it decides it on the LANDING PAD. A take /
put-back pair with a panicking operation BETWEEN the two halves, compiled in RELEASE at
codegen-units 16 and 1: today's bare `mem::take` / write-back **24 instructions, 0
`.seh_handler`, 0 personality reference**; the same through a `Ticket<T>` whose `Drop` is
`#[cfg(debug_assertions)]` **24 instructions, 0 `.seh_handler`, 2 normalised diff lines — both
the panic-location constant**; the same through a ticket whose `Drop` is ALWAYS on (the form
ERG-43's exception forbids) **37 instructions, 4 calls, 3 `ud2`, 2 `.seh_handler` and one
`rust_eh_personality` — a real landing pad.** Layout: `size_of::<Ticket<u64>>() == 4` in BOTH
profiles (the cfg'd `Drop` impl does not change size), align 4; `size_of::<Option<Ticket<u64>>>()
== 8` — there is no niche in a `u32`, so a ticket that wants to be optional wants a NonMax index
(EV-102). Behaviour: in a debug build, dropping a live ticket panics with `invariant: row 3 taken
and never returned`; `forget_ticket` is silent; `let _ = take_reserve(..)` is `error: unused
`Ticket` in tuple element 0 that must be used` under `#![deny(warnings)]`. → ZERO-COST in release,
and the always-on form is what COSTS (ERG-48 shape 4).

**EV-100 — the converse of ERG-01: what `#[repr(C)]` costs when nothing claims a layout.** (L)
One number is the whole argument. `struct { a: u8, b: u64, c: u8, d: u32 }` is **16 bytes under
`repr(Rust)` and 24 under `#[repr(C)]`** — the attribute forfeits rustc's field reordering and
adds 8 bytes, 50 %, invisibly (it never appears in a diff, only as a larger struct). On a
single-`u32` fixture the two are equal (4 = 4), which is why the habit survives: it is free
exactly where it also states nothing. Census on this checkout, no build: **230 `#[repr(C` sites
across 71 files under `crates/boyko_ecs/src`**, against **972 `const _: () = assert!` gates
workspace-wide**. Classified by whether a `const _` gate or an `offset_of!` sits within 20 lines:
**30 gated · 118 inside a `#[cfg(test)]` module · 82 neither.** Sampled in
`crates/boyko_ecs/src/ecs/core/iters/query/state.rs`: `CompA` … `CompD` (four single-`u32` test
fixtures) and `TagA` / `TagB` (two ZSTs) all carry `#[repr(C)]` and state nothing — while `P`, in
the same module, IS byte-viewed and its attribute IS load-bearing. → the attribute is NOT free in
general (ERG-01 clause 6).

**EV-101 — a zero-copy load that VALIDATES: `AlignAs` for the embedded blob, and the codegen
claim REFUTED.** (L, A, B) Layout, the half that works: `AlignAs<[u8; 64], u32>` is **align 4**
(align 8 for the `u64` witness), `offset_of!(bytes) == 0`, `size_of == 64`, while the same
`[u8; 64]` unwrapped is **align 1** — the wrapper is the whole trick, and it is a compile-time
fact rather than an observed address (the unwrapped static happened to land 4-aligned at run
time, which is exactly why the gate and not the address is the proof). Codegen, the half that
does NOT work: a summing walk over an embedded 4096-byte table read per element with
`read_unaligned` is **62 instructions with 26 vector ops (13 `vpaddd`)** — it VECTORISES; the
same walk over a validated `&[u32]` obtained after one `check_alignment` is **71 instructions
with 26 vector ops (13 `vpaddd`)**. The claim that the unaligned walk would not vectorise is
**false on rustc 1.97.1 at `x86-64-v3`**; what the validated form actually costs is nine
instructions ONCE per load (the alignment test and the length floor-division), not per element,
and what it buys is the safety property, not speed. Behaviour: a deliberately mis-aligned slice
returns `Err(Misaligned)`; a well-aligned blob with the wrong magic returns `Err(BadMagic)`; an
out-of-range id returns `Err(OutOfRange)` — never a `get_unchecked`, never UB. → ZERO-COST per
element, one check per load; the vectorisation claim is REFUTED and the rule stands on the
separating principle instead (ERG-01 clause 7).

**EV-102 — a TOP-END niche (`NonMaxU32`), and ERG-04's "one niche per type" exception CONFIRMED
rather than refuted.** (L, A, T) Layout: `size_of::<NonMaxU32>() == 4`, align 4,
`size_of::<Option<NonMaxU32>>() == 4`; `NonMaxU32::new(0)` is `Some(0)` — **index 0 stays index 0,
with no `+1` bias to state at the field** — and `NonMaxU32::new(u32::MAX)` is `None`.
WARNING — **`size_of::<Option<Option<NonMaxU32>>>() == 8`, exactly like
`Option<Option<NonZeroU32>>` == 8**: the sweep's claim that a top-end niche would make ERG-04's
"one niche per type" exception FALSE is itself wrong — `NonMaxU32` wraps `NonZeroU32` and
inherits its single niche, so the exception stands unchanged. Codegen, a probe over a 2^20 table:
`slots[i] == ABSENT` **29 instructions**; `Option<NonMaxU32>` **30**; ERG-04 shape 1's biased
`Option<NonZeroU32>` **25** — the biased form is the smallest, and the `!` in `get()` does not
survive as a hot-loop `notl`. Timing UNDER LOAD, 2^20-entry table probed 2^20 times at
golden-ratio stride, best-of-7 × 3 process runs, all three arms agreeing: sentinel
**1.278 / 1.234 / 1.201 ms**, NonMax **1.132 / 1.092 / 1.078**, biased **1.197 / 1.166 / 1.138** —
NonMax is **1.11–1.13× faster than the hand sentinel** and level with the biased form, three runs
of three. → ZERO-COST, and it removes the bias ERG-04 shape 1 concedes (ERG-04 clause 5). **ANNOTATED 2026-09-03.** The 1.11–1.13× is one 2^20-scale figure under load in the
index-width class EV-59 has since re-scaled (3–5× cache-resident, 1.3× beyond the LLC); its
direction holds, its magnitude is one point and is read as such. The layout half was re-confirmed
(`Option<Option<NonZeroU32>>` == 8 — EV-04).

**EV-103 — the safety-invariant / validity-invariant split as the `// SAFETY:` stopping rule, and
the module as the trusted base.** No compilation; the experiment IS the rule. Five blocks of
`crates/boyko_physics/src/solver/colored.rs` rewritten under the two-word template (the
`unsafe impl Send` / `Sync` for `ColorSolvePtrs`, `body_ref`, `body_copy`, `body_mut`, and the
`unsafe impl Send` / `Sync` for `ContactSolveView`): **13 clauses in total. 7 are VALIDITY facts
already discharged by a type or by the compiler** — two `BodyEffective: Copy` implies no drop glue
(the bound enforces it), three "mutation only as a single-row `base + index` write, never a
whole-buffer `&mut`" (the view type has no slice surface — ERG-07), two "address-stable base"
(the `VmReservation`) — and are therefore deletable from the prose. **6 are SAFETY facts, and all
6 name the COLORING**, which is established in a different module and is upheld by nothing inside
`colored.rs`'s own privacy — the defect the split is designed to find, and the exact obligation
EV-97's marker trait would carry as a bound. The second half, the module as the TCB, measured on
four modules as (private fields an `unsafe` block relies on) against (SAFE fns in the same module
that can write them): `component_pool.rs` **13 : 93**, `vm_column.rs` **8 : 37**,
`archetype.rs` **13 pub fields : 82**, `scope.rs` **8 private + 5 pub : 31** — four to seven safe
fns per field, so the trusted base IS the module surface and is not the 1–2 per field that would
say encapsulation is already doing the job. → COMPILE-TIME ONLY (ERG-20 clause 6).

**EV-104 — version PARITY as liveness, a third arm on EV-93.** (L, A) `SlotParity { *mut
Archetype, u32, u32 }` is **16 bytes**, the same as today's `EntityInland` and as EV-93's tagged
enum. `is_alive`: today's `!ptr.is_null() && gen == e.gen` **6 instructions**; EV-93's enum
`match` **6, and the two ICF-alias (`is_alive_today = is_alive_enum`) at codegen-units 16 and
1**; the parity arm, where a live handle's generation is always ODD and a freed slot's always
EVEN so equality alone decides, **3 instructions (`cmpl` / `sete` / `retq`) — half of both.** The
price: `free_id` **3 → 2** (the null store goes), but `alloc_id` **4 → 6** (the even-to-odd
increment). → the liveness check halves and the id arithmetic grows; like EV-93 this is an
ARCHITECTURE item for the entity-storage owner, folded into index OPEN 10, not a rule.

**EV-105 — `#![no_std]` on `boyko_utils`, priced.** (B) The three non-`sparse_map` module trees
copied VERBATIM out of the repository into a scratch crate with `#![no_std]` and no `extern crate
alloc`. First check: **11 errors** — 7 `E0433` / `E0432` "cannot find module or crate `std`" and 4
`E0223` knock-ons. After a purely MECHANICAL `std::` to `core::` substitution on seven import
lines (`fmt`, `ops`, `hash`, `hint`, `sync::atomic`): **`bit_mask` and `identifiers` compile clean
under `#![no_std]` with NO `alloc`.** The single residual blocker is `type_intern`'s
`std::sync::OnceLock` (4 sites, including `slots: [OnceLock<(K, u32)>; SLOTS]`) — `core` has no
`Sync` once-cell — and adding `extern crate alloc` **does not help at all** (the same 2 errors),
which is the fact that prices the intermediate step at zero. The two `Vec`-backed sparse maps
(7 `Vec<` in production code across `sparse_map.rs` and `sparse_slot_map.rs`) are the crate's ONLY
`alloc` users, and `sparse_map`'s `Vec<Option<usize>>` is already ERG-04's worst measured site
(EV-04 / EV-59, 2.55×) — the crate split and the ERG-04 fix are one edit. → a real, priced
migration that would make four of REF-00's five un-lintable bans un-NAMEABLE; ENGINE work, not a
rule (index OPEN 13).

**EV-106 — "push ifs up and fors down" at the named site: REFUTED.** (A, T) The
`box_sdf_manifold_avx2` pass-2 corner loop modelled at 8 lanes with a 6-offset gradient per
surviving corner. Asm at codegen-units 16 and 1: today's per-corner `if d >= 0.0 { continue }`
**127 instructions, 113 vector ops, 3 branches, 0 panic sites**; the two-pass form that compacts
the penetrating corner indices first **171 instructions, 104 vector ops, 3 branches, 1 panic
site** — the compacted form is BIGGER and LESS vectorised. Timing UNDER LOAD, best-of-7 × 3
process runs, at all three ratios that could decide it: 0/8 penetrating **0.97× / 0.97× / 1.00×**,
3/8 (realistic) **1.00× / 1.01× / 0.96×**, 8/8 **0.99× / 1.03× / 0.98×**. No win at any ratio in
any run. → REFUSED for want of a site (REF-47); the blog post may still be right elsewhere, this
site is not the evidence for it.

**EV-107 — two NIGHTLY facts this guide does not use, recorded so nobody re-derives them.** (L, B)
On `nightly-x86_64-pc-windows-gnu 1.100.0-nightly (8925ea358 2026-08-20)`, the ledger's method-S
nightly. **Pattern types**: `#![feature(pattern_types, pattern_type_macro)]` with
`use core::pattern_type;` compiles — note the spelling, `core::pat::pattern_type` does NOT
resolve — and `WorkerId(pattern_type!(u32 is 0..=63))` gives
**`size_of::<Option<WorkerId>>() == 4` AND `size_of::<Option<Option<WorkerId>>>() == 4`**: a
niche of arbitrary width, which is the one thing `NonZero` and `NonMax` (EV-102) cannot give.
WARNING — rustc emits `warning: the feature `pattern_types` is internal to the compiler or
standard library`, i.e. it is not on a user-facing stabilisation track.
**`core::mem::TransmuteFrom`**: `#![feature(transmutability)]` compiles on that nightly;
`Mat4Pod -> [u8; 64]` is accepted and `#[repr(C)] { a: u16, b: u32 } -> [u8; 8]` is
`error[E0277]: at least one value of `Padded` isn't a bit-valid value of `[u8; 8]`` — the SAME
padding proof `#[derive(bytemuck::Pod)]` already gives on stable (EV-82), and nothing more on this
shape. → both are `RUST-FRONTIER.md` rows, not ergonomics rules; this pass was scoped out of that
file and records the measurements here so the rows can be written without re-running them
(index OPEN 14).

---

## Rows added by the fourth technique sweep (2026-09-03) — the two dropped lenses, compiled

⚠️ **Where.** Rows EV-108 … EV-114 came from single-file `rustc` probes in the session scratchpad
(`erg-lab5`, outside the repository), invoked with the shipped profile's flags spelled explicitly —
`--edition 2024 -C opt-level=3 -C codegen-units=16 -C target-cpu=x86-64-v3`, no LTO — and every
identity re-run at `-C codegen-units=1`, because an ICF alias at one unit proves nothing at
sixteen. Nothing under `crates/` was built, modified or linked into any probe. The one timing
figure in this block (EV-111) was taken with the box under load, and it is a COMPILE-time figure,
not a runtime one.

**EV-108 — the `'static` bound that changes what ERG-48 shape 3's `mem::forget` hole COSTS.** (A, B)
Two spellings of the rule's own `submit` / `reclaim` sketch: `PendingA<T>` as the rule writes it
today, and `PendingB<T: 'static>` as the Embedonomicon's DMA chapter writes it, bodies otherwise
character-identical. Asm: **`drive_b = drive_a`, an explicit ICF alias at codegen-units 16 AND 1**,
15 instructions — a bound emits no code, as expected and now measured. The behaviour half is the
point. The Embedonomicon's exact hazard —

```rust
let mut local = [0u8; 64];
let t = d.submit(&mut local[..]);
core::mem::forget(t);            // the frame dies while the agent still owns the bytes
```

— **COMPILES CLEAN against the unbounded arm** (that is the falsifier ERG-48 already records, now
confirmed rather than asserted) and is **`error[E0597]: local does not live long enough` against
the bounded arm**. The bound does not stop `mem::forget`; it stops the payload from being a borrow
of a frame, so what a leak leaves behind is a live resource and not a dangling one. → ADOPTED as a
clause on ERG-48 shape 3.

**EV-109 — the SPLIT FACT: gating one copy against the other, and the forcing trap.** (L, B, A)
Three shapes, and the third is the one worth the row.

*(a) The check the derive EMITS.* A two-crate probe: `kernel` exports `pub const MAX_BUNDLE_ARITY:
usize = 16`; the "user" crate carries what a derive would emit — `const _: () = assert!(17 <=
kernel::MAX_BUNDLE_ARITY, "…")` — and the result is `error[E0080]: evaluation panicked: Bundle
arity exceeds MAX_BUNDLE_ARITY`; at 9 fields it is silent. The proc-macro crate never learns the
number and there is one copy of it.

*(b) The enum-indexed table.* `const _: () = assert!(TABLE.len() == SdfOp::COUNT, …)` plus one
per-row order assert; adding a fourth variant without a row is `E0080` naming the message.
Compile-time only, no codegen row.

*(c) THE FORCING TRAP, and it is this repository's own recorded meta-defect.* An `unsafe trait
ReadOnlyQueryData: QueryData` carrying the cross-check as a DEFAULTED associated const —
`const READ_ONLY_AGREES: () = assert!(Self::IS_READ_ONLY, …)` — over an impl that deliberately
contradicts it (`IS_READ_ONLY = false` beside `unsafe impl ReadOnlyQueryData`): **the crate
COMPILES CLEAN.** An associated-const default is evaluated only where it is USED, so the unforced
version is a gate that CANNOT FAIL. Adding one line to the generic body that every impl reaches —
`const { <D as ReadOnlyQueryData>::READ_ONLY_AGREES };` — turns the same source into
`error[E0080]: evaluation panicked: IS_READ_ONLY disagrees with the marker`, with
`note: erroneous constant encountered` naming `<Liar as ReadOnlyQueryData>::READ_ONLY_AGREES`.
Cost of the forcing line over an HONEST impl: **`drive_prose = drive_forced`, an ICF alias at
codegen-units 16 AND 1** — const evaluation precedes codegen and the line emits nothing.
→ ADOPTED as ERG-01 clause 8.

**EV-110 — `[const { X::new() }; N]` as a CHEAPER array initialiser: REFUTED.** (A) The site is
`ecs_master.rs`'s `bundle_archetype_cache`, documented as a ~30–50 µs cold path over
`Box::new(core::array::from_fn(|_| OnceLock::new()))` at `MAX_BUNDLE_TYPES = 1024`. Modelled at
that shape, `Box::new(core::array::from_fn(|_| OnceLock::new()))` and
`Box::new([const { OnceLock::new() }; 1024])` are **69 = 69 instructions with byte-identical
bodies** at codegen-units 16 and 1 (the only textual difference is the local `.LCPI` / `.LBB` label
numbering). BOTH build the array on the STACK with the same 32-wide `vmovups` loop
(`___chkstk_ms`, an 8 224-byte frame), allocate, and `memcpy` it into the box — the inline `const`
block deletes NO initialiser and produces NO `.rodata` blob. This is the EV-08 physics the
proposal's own caveat named, and it decides against it. → NOT a cost change; recorded as one
exception sentence on ERG-31 clause 1 (write whichever reads better), and NOT as a rule.

**EV-111 — "what can be `const` is `const`" has a CEILING, and it is low.** (B, T — compile time)
CTFE is a MIR interpreter. A `const fn` counting loop evaluated at compile time: **100 000
iterations is silent**; **2 000 000 iterations is `error: constant evaluation is taking a long
time`** — the `long_running_const_eval` lint, which is DENY-by-default and is therefore a hard
error out of the box. It is a LINT, so `#![allow(long_running_const_eval)]` lets it through on
stable: with the allow, 20 000 000 iterations compiles, emitting five repeated diagnostics, in
**1 m 46 s** (single file, box under load — roughly 200 k interpreted iterations per second,
orders of magnitude slower than the same loop at run time). → ADOPTED as an exception on ERG-31
clause 1: past a table of a few hundred thousand interpreted steps the answer is a build script or
a committed blob, not a bigger `const fn` — and the clause now says where it stops.

**EV-112 — the monomorphisation instrument OPEN 12 says is missing (method T).** (B) `cargo
llvm-lines` and `cargo bloat` are not installed here and are not claimed. **rustc's own collector
needs no install**: on `nightly-x86_64-pc-windows-gnu 1.100.0-nightly (8925ea358 2026-08-20)` —
the ledger's method-S nightly, run as a one-off, no channel move and no pin —
`-Zdump-mono-stats=DIR -Zdump-mono-stats-format=markdown` writes `<crate>.mono_items.md`. On a
probe with one generic fn over three types × two const-generic `bool`s it reports, exactly:

| Item | Instantiation count | Estimated Cost Per Instantiation | Total Estimated Cost |
| --- | ---: | ---: | ---: |
| `solve` | 6 | 35 | 210 |
| `drive` | 1 | 7 | 7 |

i.e. per-item instantiation counts and a size estimate — the shape of number REF-07's
"32 instantiations = 3× code" and REF-18's "+43 % build" currently rest on hand counts for.
→ Recorded as **method T** and as a partial close of index OPEN 12. It is NOT a re-measurement of
either refusal: nobody has run it over `crates/` yet, and three workflows hold that directory.

**EV-113 — OPEN 13's one residual blocker, PRICED in-house.** (B) OPEN 13 / EV-105 found that
`boyko_utils` compiles under `#![no_std]` with no `alloc` after a mechanical `std::`→`core::`
substitution, except for `type_intern`'s four `std::sync::OnceLock` sites, because `core` has no
`Sync` once-cell. The no-allocator ecosystem's answer is the third-party `static_cell`; the
in-house answer was compiled here instead. A **65-line** `OnceCore<T>` — an `AtomicU8` state
machine (`EMPTY` / `BUSY` / `READY`) over `UnsafeCell<MaybeUninit<T>>`, with `const fn new`,
`get`, `get_or_init`, `Drop`, and a `// SAFETY:` on each of its five `unsafe` blocks — compiles
under `#![no_std]` at the shipped profile with **zero `__rust_alloc` / `alloc::` / `core::fmt`
symbols in the emitted asm**. ⚠️ This is a FEASIBILITY probe, not a soundness review: a
hand-rolled once-cell sits on the boot path of every registry in the kernel and owes Miri plus a
`code-reviewer` pass before it replaces `OnceLock`. What the row establishes is only that the
blocker does not require a dependency. → index OPEN 13, updated.

**EV-114 — phantom SOURCE / DESTINATION spaces on a transform: free, and refused anyway.** (A, L)
`euclid`'s `Transform3D<T, Src, Dst>` modelled on this engine's `Mat4`:
`Tf<Src, Dst>(Mat4, PhantomData<fn(Src) -> Dst>)` with a `then<Out>(self, Tf<Dst, Out>)`
composition, against the bare `proj.mul(view)` the tree writes at
`boyko_render/src/view.rs::view_proj_columns`. **`view_proj_spaced = view_proj_bare`, an ICF alias
at codegen-units 16 AND 1**, 17 instructions; `Tf<World, View>` is 64 bytes / align 16, identical
to `Mat4`. The encoding costs nothing at run time, which is why the refusal that follows is a
Part-B one: it is the API surface that pays — two parameters on every constructor, every `Mat4`
method and every GPU upload seam, across 276 space-named matrix occurrences in 13 files, in a
crate whose math is bit-determinism-pinned against a shader oracle. → REF-50, with ERG-02's
one-fact newtype named as the cheaper answer at the site that motivated it.

---

## Rows added by the re-verification and the profile investigation (2026-09-03)

**Why this block exists.** The owner read a summary of this ledger and said the numbers looked
wrong. They were: rows EV-01 … EV-58 were taken at `codegen-units = 1` while the shipped build uses
16; several headline magnitudes (EV-02's 1.5×, EV-21's 1.7×, EV-22's 25 %, EV-90's 8–9×, EV-92's
0.76–0.79×) were L1-resident artefacts or did not reproduce at all; EV-60's re-take had one fixture
at one scale; and the brief that commissioned the re-verification itself repeated a stale ISA
sentence. Three independent lenses re-took the contested rows — TIMING at three cache tiers,
IDENTITY at an explicit 16 and 1 codegen units with forced unit splits, and ENGINE-SHAPE realism on
fixtures read out of the tree — and a fourth pass measured what the missing `[profile.release]`
and the ISA setting are actually worth. The re-verdicts are written INTO the rows above (old
verdict struck through, dated, with the reason); the rows below carry the method, the instruments'
defects, and the profile investigation's own numbers.

**Where.** Four labs, all outside the repository, none of which built, modified or linked anything
under `crates/` (three other agents held that directory throughout): `erglab6` (timing lens — the
session scratchpad; own `rust-toolchain.toml` pinning `stable-x86_64-pc-windows-gnu` 1.97.1, own
`.cargo/config.toml` with `-C target-cpu=x86-64-v3`, `[profile.release]` `opt-level = 3` /
`codegen-units = 16` / `lto = false` / `panic = unwind` / `debug = false`, and a byte-identical
sibling profile `release-cu1`); `erg_idlab` / `erg_idlab2` / `erg_defprof` / `erg_beh` (identity
lens — `C:/Users/flint/AppData/Local/Temp/`; the second lab carries 144 filler fns so the
partitioner really splits a probe pair, cu=16 vs cu=1 set by `CARGO_PROFILE_RELEASE_CODEGEN_UNITS`);
`shapelab` (engine-shape lens — the session scratchpad, same pinning); and the profile
investigation's seven bench-binary configurations under `/d/tmp/isabins/{A,A0,B,C,D,E,F}`, built
from ONE verified-identical source snapshot (sha1 over `crates/{boyko_physics, boyko_ecs,
boyko_threadpool, boyko_utils, boyko_math}/src` plus the three bench files, identical before the
first build and after the last) with `cargo bench --no-run` into private `CARGO_TARGET_DIR`s, so the
other agents' incremental state in the worktree's `target/` was never touched. Machine: AMD Ryzen 9
5900HS (Zen 3, 8C/16T, 32 KiB L1d and 512 KiB L2 per core, 16 MiB shared L3, no AVX-512) — the
owner's workstation, with other agents building throughout. graphify is not installed on this box;
every in-tree shape was read by grep and Read.

**EV-115 — what `cargo build --release` IS on this checkout, from the rustc line.** (B) Every
profile key is ABSENT from every manifest: `grep -rn '^\[profile' --include=Cargo.toml` over the
workspace returns exactly one hit, `[profile.bench]` / `codegen-units = 1` in the root `Cargo.toml`
(whose own comment, "Phase X.E — deterministic bench codegen", says it is deliberately NOT `lto`);
there is no `[profile.release]` in the root or in any of the 28 member manifests, no `opt-level` /
`lto` / `panic` / `debug` / `overflow-checks` / `incremental` / `strip` key anywhere, and no
`CARGO_PROFILE_*` in `.github/workflows/*.yml`. Confirmed from the rustc command line of
`cargo build --release -p boyko-math -v` (captured at `/d/tmp/verbose.log`): `-C opt-level=3`;
NO `-C codegen-units` flag (rustc's non-incremental default of 16 applies); `-C embed-bitcode=no`
(cargo's encoding of `lto = false`); no `-C panic=` (unwind); no `-C debuginfo` /
`-C debug-assertions` / `-C overflow-checks` / `-C incremental` (all off); and `-C strip=debuginfo`,
which cargo 1.97 emits by default when `debug = 0` — the shipped release is already stripped of
debuginfo without anyone asking. ISA: `-C target-cpu=x86-64-v3` from the three `[target.x86_64-*]`
tables of `.cargo/config.toml` (landed in `cced895a`, amended in `af6e7043`), mirrored three times
in `ci.yml`'s `RUSTFLAGS` precisely because `RUSTFLAGS` REPLACES config rustflags, and guarded by
`tests/isa_baseline_census.rs`'s `the_declared_isa_baseline_actually_reached_the_compiler`. No
`-C target-feature` anywhere; the v3 level supplies avx2 + fma + bmi1 + bmi2 + f16c + lzcnt + movbe
and selects the matching scheduling model. Machine-local and load-bearing: `$CARGO_HOME/config.toml`
adds `-Cdlltool=…/llvm-dlltool.exe`, `-L …/.rust-tools/lib` and `-Clink-arg=-B…/bin-lld` for the gnu
triple, and cargo JOINS rustflags arrays across config files, so the effective release build carries
four flags, not one. → this is the profile every 2026-09-03 row names, and the profile every
earlier row is measured against.

**EV-116 — AVX2 IS in the shipped physics rlib, and was not before `cced895a`.** (A, structural,
2 clean builds, byte-identical rlib size.) `cargo build --release -p boyko-physics` at the shipped
profile into a private target dir, then `llvm-objdump -d` / `llvm-nm --defined-only` from the gnu
sysroot over `libboyko_physics.rlib` (3 005 400 bytes): **3 620 `ymm` register references, 344
`vmulps …ymm`, 329 `vpbroadcast` / `vbroadcastss`, 2 `vpermilps`, and ZERO `vfmadd` of any form**
— matching the determinism contract in `.cargo/config.toml`'s own comment (Rust does not contract
`mul` + `add` without an explicit `mul_add`). All nine `cfg(target_feature = "avx2")`-gated symbols
are present: `solver::simd::{apply_gravity_avx2, refresh_inertia_avx2, position_integrate_avx2,
pointvel_x8, effective_mass_x8, apply_impulse_blend_x8, dot8}`, `sdf_simd::sdf_edit_list_x8` and
`ColoredSoftStepSolver::solve_color_avx2`. CONTROL, same tree, same command, `RUSTFLAGS` carrying
the three machine-local flags plus `-C target-cpu=x86-64` (the pre-`cced895a` state): rlib 2 824 372
bytes, **0 AVX2 symbols, 0 `vmulps …ymm`, 753 SSE `mulps`**. → the flag does real work; the AVX2
solver ships today and did not before. (The first control attempt with `RUSTFLAGS='-C
target-cpu=x86-64'` alone died with `error calling dlltool 'dlltool.exe': program not found` —
EV-122's third defect — and the attempt via `cargo --config 'target.<triple>.rustflags=[]'`
produced a byte-IDENTICAL rlib to the unmodified build, caught only by comparing sizes.)

**EV-117 — what the ISA setting is WORTH: 1.97× on the AVX2 colored solve, 1.10× on the scalar
solve, nothing on the ECS storage path.** (T; config A = today's release codegen through the bench
harness — `CARGO_PROFILE_BENCH_CODEGEN_UNITS=16`, `CARGO_PROFILE_BENCH_LTO=false`, `x86-64-v3`;
A0 = identical with `-C target-cpu=x86-64`; 8 interleaved passes at ≈13-second granularity, the
concurrent `rustc` / `cargo` count logged per pass and 0 for 39 of 45 logged slots; criterion
`--warm-up-time 1 --measurement-time 2 --sample-size 15`.) `boyko_physics/benches/colored_solve.rs`,
`o7_simd_ab/simd_solve_single/10200` — `solve_color_avx2` against the scalar oracle on a warmed
10 200-contact resting pile (128 × 40), single-threaded, no schedule, no threadpool: **A median
4.877 ms (IQR 3.6 %, 4.729–5.074) vs A0 9.474 ms (IQR 2.8 %, 9.392–11.146) — per-pass ratio median
1.970, range 1.908–2.254**. CONTROL, `scalar_colored_single/10200` (same scene, no AVX2 kernels):
A 8.868 ms (IQR 7.5 %) vs A0 9.624 (IQR 18.8 %) — 1.10×, range 0.859–1.324, i.e. autovectorisation
of scalar code only; an ISA delta of zero there would have falsified the measurement and it did
not. `boyko_ecs/benches/component_pool_dense.rs`, `swap_remove/10000`: A 63.09 µs vs A0 61.19 —
**0.972×, NO ISA effect**. Working sets: the 10 200-contact scene is ≈2–3 MiB (5 120 `BodyState` of
≈150 B plus 10 200 manifolds) — L3-resident, past the 512 KiB L2; nothing here spills the LLC, so a
DRAM-bound verdict is NOT supported by this row. → the owner's 2026-09-02 ISA ruling was taken
without a number, and this is the number: 1.97× on the solver, ≈10 % on the rest of physics, 0 %
in the ECS, at the documented portability price (illegal instruction on anything older than
Haswell 2013 / Excavator 2015).

**EV-118 — `[profile.release]` candidates on the ECS storage and query hot paths, six configs,
seven benchmarks.** (T; same harness and discipline as EV-117; configs A = cgu16 / no LTO (today),
B = cgu1, C = `lto = "thin"` cgu16, D = `lto = "fat"` cgu16, E = `target-cpu=native`, F = cgu1 +
`lto = "fat"`; set with `CARGO_PROFILE_BENCH_CODEGEN_UNITS` / `CARGO_PROFILE_BENCH_LTO` plus
`RUSTFLAGS` for E.) Per-bench medians and ratio to A:

| Bench (crate / file / id) | A | B cgu1 | C thin | D fat | E native | F cgu1+fat |
|---|---|---|---|---|---|---|
| `component_pool_dense` `swap_remove/10000` | 63.09 µs | 0.986 | **0.645** | **0.648** | 0.978 | **0.624** |
| `component_pool_dense` `add fill/10000` | 73.77 µs | 1.010 | 0.912 | **0.850** | 1.022 | **0.831** |
| `query_dsl` `query_mut_iter_10k` | 7.94 µs | **0.721** | 1.004 | 1.015 | 1.009 | **0.750** |
| `query_dsl` `query_tuple_2_ref_iter_10k` | 8.46 µs | 0.917 | 0.991 | 1.004 | 1.002 | 0.949 |
| `query_dsl` `query_ref_iter_10k` | 26.91 µs | 0.999 | 1.000 | **0.373** | 1.009 | **1.174** |
| `colored_solve` `simd_solve_single/10200` | 4.877 ms | 0.96–1.01 | ≈1.0 | ≈1.0 | ≈1.0 | 0.94–1.01 |
| `colored_solve` `scalar_colored_single/10200` | 8.868 ms | ≈1.0 | ≈1.0 | ≈1.0 | ≈1.0 | ≈1.0 |
| **geomean, all seven** | 1.000 | 0.933 | 0.922 | **0.787** | 0.992 | 0.873 |
| **geomean without `query_ref_iter_10k`** | 1.000 | 0.922 | 0.910 | 0.891 | 0.990 | **0.831** |

IQRs: A 0.6–7.7 %, B 1.5–10.0 %, C 0.6–4.6 %, D 1.1–4.0 % except the bimodal rows below, E 0.7–1.9 %,
F 0.4–8.8 %. Three findings, and they are NOT one lever. (1) **LTO is the ECS-storage lever**:
`swap_remove` 0.645× thin / 0.648× fat (−35 %), `add fill` 0.912× / 0.850× — cross-crate calls
(bench target → `boyko_ecs` rlib), structurally the boundary `boyko_app` → `boyko_ecs` crosses in the
shipped binary; `codegen-units = 1` alone does NOTHING there (0.986×). (2) **`codegen-units = 1` is a
DIFFERENT lever**: `query_mut_iter_10k` 0.721× (−28 %), `query_tuple_2` 0.917×; LTO does nothing on
those. Complementary, not substitutes. (3) **Neither does anything for the physics solver**
(0.94–1.01×): those hot loops are intra-crate and already fully inlined at 16 units. ⚠️ **ONE
NON-MONOTONIC RESULT, reported rather than smoothed:** `query_ref_iter_10k` is **2.68× FASTER under
fat LTO at 16 units** (26.91 → 10.03 µs; 6 of 8 passes ~10.0 µs, 2 contended passes 21–25 µs — the
median and min stable at 9.91–10.04) and **17 % SLOWER under fat LTO at 1 unit** (31.58 µs, IQR
0.4 %). Same optimisation, opposite sign, decided by the unit count; both measurements have IQR
under 1 % over 8 interleaved passes, so this is not noise. It was NOT disassembled; until someone
does, config F is known to leave a 3.1× on the table on one measured ECS path, and D beats F there
while losing 5–10 % on four others. All seven benchmarks are single-threaded by choice
(`*_parallel_4w`, `parallel_solve`, `broadphase` — under active edit — and every KE16 bench were
excluded, and `bench_bevy_vs_boyko` across seven configs was not affordable), so nothing here says
what LTO or the ISA do under the scheduler. → the levers and their numbers are in the index's
profile section; the choice is the owner's.

**EV-119 — `-C target-cpu=native`: twelve features nobody uses, 0.8 %.** (B, T) `comm -23` over
`rustc --print cfg -C target-cpu=native` against `-C target-cpu=x86-64-v3` on this Zen 3 box:
native ADDS `adx, aes, pclmulqdq, rdrand, rdseed, sha, sse4a, vaes, vpclmulqdq, xsavec, xsaveopt,
xsaves` and removes none. None is used by this engine's hot code: config E's per-bench ratios to A
are 0.976, 0.984, 0.982, 1.008, 0.995, 1.010, 0.992 — every one within ±2.5 %, geomean 0.992,
inside the noise band — for a build that becomes machine-dependent. → REFUSED as a setting; the
number is the reason.

**EV-120 — what the candidate profiles COST: clean build time and binary size.** (T-build, size;
two samples each for A / C / D / F taken ≈2 h apart under different background load, one sample
for B / A0 / E.) Clean build of the three bench binaries plus their ≈20-crate dependency closure,
fresh `CARGO_TARGET_DIR` each time: **A (cgu16, no LTO) 148 s / 100 s · C (thin) 167 s / 117 s ·
D (fat, cgu16) 233 s / 324 s · F (cgu1 + fat) 257 s / 316 s** · B (cgu1) 221 s · A0 194 s · E
158 s. The two samples of A differ by 48 %, so these are an order-of-magnitude cost, not a precise
one: fat LTO costs roughly 2–3× the clean build, thin ≈+15 %. Two later build-time re-samples
FAILED mid-edit with exit 101 after `boyko_threadpool`'s source hash changed under the pass
(5f82df9f → 44afdf05); they are reported, not dropped, and no TIMING in EV-117 / EV-118 was taken
after that change. Emitted binary sizes were byte-identical across samples, so the builds
themselves are deterministic. Binary size, `colored_solve.exe`: **A 8 760 832 B (1.00×) · A0
8 631 296 (0.985) · B 6 421 504 (0.733) · C 5 450 752 (0.622) · D 3 457 024 (0.395) · E 8 812 032
(1.006) · F 3 326 976 (0.380)** — LTO SHRINKS the binary 38–62 %; it does not grow it. Release
rlib, `boyko_physics`: defaults 3 005 400 B in 14 s / 24 s; `CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1`
+ `CARGO_PROFILE_RELEASE_LTO=fat` 3 153 146 B in 19 s / 21 s — an rlib GROWS ≈5 % under LTO
(embedded bitcode) and costs almost nothing to build; the LTO work and the size win both land at
the FINAL link, i.e. on every binary and every test target, and barely on an incremental rlib
rebuild. → build time is the only cost; binary size is a win; portability is unchanged (LTO and
codegen-units change no ISA and no ABI).

**EV-121 — the DISCARDED first campaign, and a verdict RETRACTED.** (T) Six full rounds, 22
benchmark ids, 7 configs, 924 data points, criterion `--measurement-time 3 --sample-size 20`,
configs interleaved at ~2.2-minute granularity: per-(id, config) spreads of **43 % to 809 %**, and
rounds 1–3 systematically 25–35 % slower than rounds 4–6 for EVERY config — machine drift under the
other agents' builds (recorded concurrent `rustc` / `cargo` count 0 for round 4, 2–24 for rounds 5–6),
not a config effect. NOT USED for any verdict. Its apparent finding that `codegen-units = 16` costs
1.45× on the AVX2 solve did NOT survive the tight re-take (0.96–1.01×, EV-118) and is RETRACTED
here. → the spread IS the finding; a campaign whose cells swing 8× is a record of the machine, not
of the code, and the second campaign (8 passes at 13-second interleave, IQR typically under 4 % and
as low as 0.4 %) is the one every number above comes from.

**EV-122 — the instruments' three defects, each of which produced a wrong-looking-right result
before it was caught.** (B) (1) **A bare `--emit=asm` measures ONE codegen unit.** On `erg_idlab2`:
`[profile.release] codegen-units = 16` + `cargo rustc --release --lib -- --emit=asm` → 16 `.s`
files, 3 alias lines; the SAME command with that profile line DELETED (cargo passes no flag — the
workspace's exact situation) → 1 `.s` file, 13 alias lines; while a plain `cargo build --release`
of the same crate with no flag → THREE `cgu.*.rcgu.o` members in the rlib (listed by parsing the
archive; `ar` is not on this box). A single-file `rustc --emit=asm` prints `warning: resetting to
default -C codegen-units=1` and ignores a requested 16. So the shipped build is genuinely
multi-unit, and every ledger row whose asm came from a bare `--emit=asm` observed one unit while
believing it observed the default. (2) **`cargo --config 'target.<triple>.rustflags=[]'` does not
OVERRIDE the file, it JOINS to it**: the control build so spelled finished green and produced a
byte-IDENTICAL rlib to the unmodified build (3 005 400 B, 3 620 `ymm` refs); cargo merges config
arrays by concatenation, so an empty array is a no-op — a pass that had used it would have
reported "no ISA difference" over the same binary twice. (3) **`RUSTFLAGS` REPLACES config
rustflags, and on this machine that breaks the BUILD, not just the codegen**: `RUSTFLAGS='-C
target-cpu=x86-64'` alone died twice with `error calling dlltool 'dlltool.exe': program not found`,
because `$CARGO_HOME/config.toml`'s three machine-local flags were discarded with the ISA one;
every `RUSTFLAGS`-based configuration in EV-116 … EV-120 carries those three explicitly, and a
reproduction on another box must re-derive them. The same replacement is why CI spells
`-C target-cpu=x86-64-v3` three times, and it is a standing hazard: any agent or CI leg that sets
`RUSTFLAGS` without it ships the scalar fallback again, silently, and only
`tests/isa_baseline_census.rs` would say so. → method A now names the unit count on every identity;
method T names the override variable on every configuration.

**EV-123 — the noise floor over IDENTICAL machine code, and a sign that flipped with placement.**
(T) The ICF-aliased pair `f32_seq_unchecked = f32_seq_checked` (one symbol, EV-02) timed as if it
were two variants across four separated runs: L1 0.811 [0.733..0.816] vs 0.798 [0.736..0.807]
ns/elem (2 %), L2 0.862 [0.759..0.876] vs 0.814 [0.796..0.952] (6 %), 64 MiB 2.202 [1.484..2.795] vs
2.747 [1.212..2.936] (25 %). Those are this box's noise bands under this load, and every 2026-09-03
ratio inside them is reported as no claim. Second instance: EV-22's `f32` split ratio was raw
1.5× faster at L1 in binary `hot` and slice 1.4× faster in binary `hot2` — same source, same
profile — and EV-59's 512-key sparse ratio moved from ≈1.0 to ≈2.0 when ONE scale was added to the
probe's scale list, without either measured function's asm changing. Contamination log, named not
dropped: `hot` run 1 at 80 → 100 % load with ≈18 concurrent `rustc`; run 3 at 28 → 100 %; `allocp`
run 1 at 70 % produced a 2.4× offset on one arm (EV-45); `shapelab` pass 1 of every probe at
73–100 % load with 6–18 build processes, absolute figures discarded; `shapelab` probe C at 786 432
contacts in passes 2 and 4 (2.33 and 5.14 against 1.03–1.11) discarded as single-arm stalls;
`shapelab` probe D at 1 048 576 keys in passes 1 and 2 (1.00 and 0.97 against 6.43–10.64) kept and
REPORTED as the L3-evicted regime; the identity lens ran with 3–8 concurrent `cargo` / `rustc`
throughout and its 2^20 sparse point swung 26× (EV-59). Absolute ns/elem beyond the LLC swings
2–3× with load throughout, and no absolute figure at 64 MiB is quoted anywhere in this block. → the
second reading rule (EV-49) is vindicated hard, and the fourth reading rule is its consequence.

**EV-124 — the devirtualisation trap: a `dyn` probe that measured a malloc, and a `dyn` probe
that measured nothing.** (A, T) Written as EV-13 and EV-60 describe theirs — one concrete iterator
type behind `Box<dyn Iterator>` — LLVM DEVIRTUALISED it at the shipped profile: `sum_boxed`'s loop
is `vaddss (%rcx),%xmm6,%xmm6; addq $64,%rcx; cmpq; jne`, byte-identical to the RPIT loop, no
indirect call; only one `__rust_alloc` and one `___rust_dealloc` survive, and the "cost" is one
malloc+free per call amortised over n. A box minted and consumed in ONE function in `shapelab`
devirtualised so completely that the two arms read 0.68 vs 0.68 ns/elem — a null result that means
"the fixture leaked", not "the box is free". Only a box that may hold either of TWO concrete types
behind an opaque flag (`movq %rsi,%rcx; callq *%rbx; testb $1,%al` in the loop) or one crossing an
`#[inline(never)]` seam shows a vtable call. Second trap in the same family: an `f32` reduce is
latency-bound, so its serial accumulator chain hides a whole indirect call (EV-13's 1.17× / 1.05× /
0.89×), and had to be rebuilt as an integer fold before it could show anything. → the fifth reading
rule; EV-13's and EV-60's `Box` figures are read under it.

**EV-125 — the engine's shapes, read out of the tree, and the cache regime each sits in.** (read,
no build) Component sizes from the tree's own `const _` gates: `Visibility` 1 · `MeshHandle` /
`Name` 4 · `UiText` 12 · `UiAnchor` / `ParticleEmitter` 16 · `UiImage` / `UiWorldProjection` 24 ·
`ParticleRender` 32 · `Transform` 40 · `UiBackground` 44 · `GlobalTransform` / `ParticleSim` /
`InstanceModelCol` / `TrsPacked` 48 · `Gpu3dInstance` 52 · `UiInstance` / `VbInstanceRow` /
`BodyEffective` 64 (`f32 + Mat3 + Vec3 + Vec3`, align 4 — NOT 16) · `Manifold` 152. **The most
iterated components are 12–48 B; 64 B is the top of the range and the physics shape**, and
`boyko_ecs`'s own `query_iter.rs` bench iterates a 12 B + 12 B pair. A query fetch also touches a
per-row 4-byte `Tick` (`component_pool.rs` pins `size_of::<UnsafeCell<Tick>>() == 4`) across two or
three columns. Storage: `POOL_MIN_ROWS = 65_536`, `POOL_MAX_ROWS = 16_777_216`, `COMMIT_GRANULE` =
`POOL_MIN_SLAB` = 64 KiB (`ecs/constants.rs`) — a 64-byte column at `POOL_MIN_ROWS` is 4 MiB, past
L2, inside L3. Parallelism: `MIN_ARCHETYPE_FOR_PARALLEL = 1024` (`par_iter.rs`),
`BatchingStrategy::default()` = `entity_count / (16 workers × 1)` clamped to ≥ 1024 — so the chunk
is 1024 rows at 4 096 and at 10 000 entities, 4096 rows at 65 536, 6250 at 100 000, and **the
PER-WORKER working set is 12–256 KiB (L1/L2) at every population the engine benches**; the
beyond-LLC regime is reachable only by a single-threaded whole-column walk over ≥ ≈250 k rows,
which no bench in the tree does. Physics: the 10k pyramid is base 141 → 10 011 bodies / 29 751
contacts (bodies 640 KiB, the 10 200-contact `o7_simd_ab` scene ≈2–3 MiB); `solve_color_avx2` is
COHORT = 8 lanes gathering `body_a` / `body_b` per slot through `ScratchSolveView::row_ptr`; the
benches run 1k / 10k / 100k bodies. `SparseMap` key domains: `component_pool_bundle::sparse_indexes`
is keyed by `ComponentId < MAX_COMPONENTS = 512`; `archetype_registry::id_to_location` by
`ArchetypeId`; only `clone::map` and `observers::entity_store` are entity-keyed and can reach
10^5–10^6. → the three-point scale of every 2026-09-03 timing (16 KiB / 256 KiB / 64 MiB) brackets
these; the engine's own regime for a parallel system is the L1/L2 pair, where the per-element costs
are LARGEST, and for `SparseMap` it is the 512-key table, where the width win is SMALLEST.
