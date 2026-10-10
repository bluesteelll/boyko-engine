# RUST-FRONTIER — the standing answer to "should we use the new thing" in boyko-engine

**The owner's ask, verbatim (2026-09-02).** *"I mean it generally, that you use the coolest and
freshest capabilities of Rust. For instance I think we could move to the new trait solver now."*
And, the same day: *"no, we can use nightly too."*

**What this document is.** [RUST-ERGONOMICS.md](RUST-ERGONOMICS.md) is the rules for code being
written NOW on the pinned stable channel — every one of its rules is stable-only by construction.
This document is its companion in the other direction: for each capability recent Rust has added
(stable 1.85 → 1.98, plus the nightly features near enough to matter), the standing verdict on
whether THIS engine takes it, at what price, and what it would buy. The ergonomics guide answers
"how do I spell this"; this one answers "may I use that yet". A feature that this document
ADOPTS becomes, once converted, a rule or an exception in the ergonomics guide; until then it is
a site to convert, named below.

**The question is not "what is new in Rust".** It is *what does new Rust buy this engine, at what
price, and which of it can we have on stable*. Three verdicts, and each entry carries exactly one:

- **ADOPT** — a real site in this tree AND a cost verdict that was COMPILED on this box, never
  assumed. Where the crate holding the site is under edit by another workflow, the entry says
  "convert LATER" and names the file and symbol; this pass changed no production code.
- **REFUSE** — with the reason. A feature that is genuinely interesting and buys this engine
  nothing belongs here, not in a footnote; this section is as substantial as the adoption section
  because on a codebase that is already on the frontier of what matters to it, refusal is the
  common correct outcome.
- **BLOCKED-ON-STABLE** — what we would take the day it stabilises, with the tracking issue,
  what to watch, and the STABLE STAND-IN named beside it — the thing to write today that costs
  the same and converts mechanically when the gate opens.

**Nightly, and the one condition.** Nightly is PERMITTED (owner ruling, 2026-09-02), so a
nightly feature is adoptable; its price is the PIN, and the entry must state it: which exact
nightly, whether the whole workspace or one crate needs it, and what has to be re-checked when the
pin moves. **A floating nightly is never acceptable** — `channel = "nightly"` in a
`rust-toolchain.toml`, or `dtolnay/rust-toolchain@nightly` in CI, turns an unrelated compiler
change into a surprise red on a Tuesday. A pinned nightly is `channel = "nightly-YYYY-MM-DD"` in
`rust-toolchain.toml`, moved deliberately, with the re-check list in §5 run on the day it moves.
This tree already pins one (`.github/workflows/docs.yml`, `nightly-2026-05-20`, with a written
reason) and already FLOATS two (§5) — so the condition is not hypothetical here.

**Per-crate nightly does not exist.** There is no `rust-toolchain.toml` at the repository root;
rustup selects a toolchain per invocation and per directory, and `cargo build --workspace`
compiles every member with ONE rustc. A nightly feature in `boyko_physics` moves the whole
workspace — every gate in CLAUDE.md, every `.stderr` fixture — onto that nightly. Every "one
crate" price below is really a workspace price, and the entry says so.

**Precedence.** CLAUDE.md's principles win any conflict; the ergonomics guide's rules win any
spelling question. Code is cited by file path and item name, never by line number — the tree
moved under this pass (three crates dirty in `git status` throughout), and a line number would
already be wrong.

**Method** (house format, [rust-ergonomics/EVIDENCE.md](rust-ergonomics/EVIDENCE.md)).
Toolchains: `rustc 1.97.1 (8bab26f4f 2026-07-14)` `stable-x86_64-pc-windows-gnu` — the engine's
channel, confirmed on the first command of the pass; `rustc 1.98.0 (88d9e12ae 2026-08-18)`
installed side by side as a NAMED toolchain (the channel was deliberately not moved); `rustc
1.100.0-nightly (8925ea358 2026-08-20)` `nightly-x86_64-pc-windows-gnu`. Every probe carried
`-C target-cpu=x86-64-v3` explicitly, because a `RUSTFLAGS` variable REPLACES the
`.cargo/config.toml` value rather than appending to it and a probe that drops it measures a
different engine (`tests/isa_baseline_census.rs` exists for exactly that). Labs are single-file
`rustc --emit=asm` probes in the session scratchpad and scratch crates under the system temp
directory, built at `codegen-units = 1` for identical-code folding AND re-built at the shipped
16 (via `--out-dir`, because `--emit=asm -o` silently resets to 1 — recorded because the first
attempt did exactly that); nothing under `crates/` was modified, and the workspace builds cited
from the probes went into scratch `CARGO_TARGET_DIR`s. Methods per row: **L** layout gate, **A**
assembly, **B** behaviour (a deliberate compile-fail or a runtime print), **S** structural (a
count over the tree), and one this ledger adds — **V (version gate)**: the same source compiled
on each toolchain to establish exactly where a feature is gated, quoting the `E0658` issue
number the compiler prints. **No timing verdict appears in this document.** The only wall-clock
numbers are run receipts from the probes, taken under load (a game, two browsers, three sibling
agent sessions and the other workflow's clippy runs were resident), labelled as such where
cited, and never used to decide anything. Rows are `FV-nn` in §7.

**Size.** Five adoptions (FR-01 … FR-05), sixteen refusals (FR-10 … FR-25), four blocked-on-stable
entries (FR-30 … FR-33), fifteen evidence rows (FV-01 … FV-15), and five defects found in passing
(§6) that are not adoption questions at all and are the most valuable thing in the file.

---

## §1 The trait solver — the owner's question, answered directly

**What it is.** The next-generation trait solver is a rewrite of the part of rustc that proves
`T: Trait` obligations and normalises associated types — the phase that decides whether a
program type-checks and how long that takes. It selects the same impls as the old solver by
design; it does not touch codegen, so it cannot change the machine code of a program it accepts.
It is controlled by ONE nightly-only flag with THREE values, and the compiler names them itself
(FV-10): `-Znext-solver=globally` (the whole type checker), `=coherence` (only the overlap check
between impls — and this is the compiled-in DEFAULT), `=no` (the escape hatch that turns even
that off).

**Half of it is already yours.** `coherence` is the default on the compiler this project pins —
read off the option table of 1.97.1, 1.98.0 and the nightly alike (FV-10) — and the Rust 1.84
release notes list "Use the next-generation trait solver in coherence" as shipped. The `=no`
value exists only to switch it off. For that half, "move to the new trait solver" is already
done and has been shipping on stable since 1.84; there is nothing to adopt and nothing to pay.

**The other half — `=globally` — is the actual proposal, and it was measured against this tree
(FV-11).** It ACCEPTS the entire workspace and every third-party dependency: `cargo check
--workspace --all-targets` under `-Znext-solver=globally`, `EXIT=0`, zero errors. It rejects
nothing the old solver accepted. It makes nothing faster at runtime, by construction. Compile
time could NOT be measured on this box — the noise floor for the same configuration was 265 s vs
299 s, so no honest number exists, and nothing observed pointed at "faster".

**What it buys, exactly one thing, and it is real.** Under `=globally` the solver reports that
proving `RenderResources: Send` and `: Sync` (`crates/boyko_demo/src/app.rs`, the struct in
`crates/boyko_demo/src/render/mod.rs`) overflows the recursion limit, under the
future-incompatible lint `recursion_depth_exceeding_limit` (rust-lang/rust#159228, "will become
a hard error in a future release"). Every field of that struct is a `wgpu` type; the depth is
wgpu's graph, not ours. But it is a break scheduled to arrive on stable on someone's Tuesday, it
is INVISIBLE to today's default build, and one crate attribute buys it off (FR-03, measured to
zero).

**What it costs, and why it cannot be THE build.** Of 72 `boyko_ecs` compile-fail fixtures
replayed one at a time, 64 produce byte-identical stderr, 3 shift a `required by this bound`
span, and 4 go from ONE error to SEVEN — the hand-authored `#[diagnostic::on_unimplemented]`
message for `ChunkedQueryData` (`crates/boyko_ecs/src/ecs/core/iters/query/chunked_data.rs`) is
printed once by the old solver and three times by the new one, plus a spurious closure-bound
`E0277` and an `E0271` the old solver suppressed. No fixture flipped from reject to accept — the
gate has no hole — but this tree pins 134 `.stderr` files across 20 trybuild suites, and a
fixture re-blessed under `=globally` is red on stable: **the suite cannot be green on both
solvers at once.** This project's own history has a fixture that sat red for 87 commits because
its `.stderr` was never re-blessed. Adopting the new solver as the build would make that a
scheduled obligation on every pin move, for a compile-time change nobody could measure and a
runtime change that does not exist.

**Recommendation.** Take it in the only form that costs nothing: a SECOND, NON-BLOCKING CI leg
(FR-04) that runs the workspace under `-Znext-solver=globally` on a nightly PINNED BY DATE,
`continue-on-error: true`, its output read by a human and consumed by nothing — so no `.stderr`
is re-blessed and nothing on stable moves. Take the one thing it found today (FR-03). Do NOT pin
a nightly to make it the build: the team's own announcement (2026-08-21, the day after the
installed nightly) says it is on by default on nightly now and they "plan to stabilize it in the
next months", so a pin buys only the interval until it arrives on the stable channel for free —
and when it does, the fixture re-bless happens ONCE, the same day the channel moves (FR-32). The
owner's sentence is right in substance and early by one stabilisation: the solver is coming to
you; you do not need to go to it.

---

## §2 ADOPT

Each entry: the feature and where it is available · the site in this tree · what it buys · the
cost verdict with the method and the `FV` row · the price of the pin, where one applies · the
conversion status.

**FR-01 — `core::range::Range<usize>` for a `[lo, hi)` span that is passed down and then re-read.**
Stable 1.96 (`#[stable(feature = "new_range_api", since = "1.96.0")]`, FV-07); compiles on the
pinned 1.97.1. ERG-02 already says a `[lo, hi)` pair is a `Range<T>` and EV-74 compiled the
conversion of `crates/boyko_physics/src/solver/colored.rs`'s `span: (usize, usize)` to
`core::ops::Range<usize>` at zero cost — but recorded it as a SHOULD, and what EV-74 did not price
is that `core::ops::Range` is `!Copy` (FV-07: `is_copy::<ops::Range<usize>>()` → `E0277`). At the
real site the span is passed by value into `solve_color`, then re-read for the slot count
(`span.1 - span.0`) and the chunk-containment check (`chunk_start >= span.0 && chunk_end <=
span.1`), then passed AGAIN — the shape that, spelled with `ops::Range`, is `error[E0382]: use
of moved value: span` (FV-07) and forces a `.clone()` at every hand-off. `core::range::Range` is
`Copy` (`#[derive(Clone, Copy, PartialEq, Eq, Hash)]` in `core/src/range.rs`), implements
`SliceIndex<[T]>` so `&xs[span]` hoists the bounds check, `IntoIterator`, `is_empty`,
`contains`, and `From` in both directions with `ops::Range`. **Buys:** the order stated at
construction, saturating `is_empty`, the slice-index form, and a span that copies like the tuple
it replaces — ERG-02's `Range` clause without the friction that kept the site a tuple.
**Cost:** ZERO-COST (A, FV-08): the solve loop over the tuple, `ops::Range` and
`core::range::Range` is 17 = 17 = 17 instructions with 0 normalised-diff lines; the dispatch
shape (pass down, re-read count and containment, pass again) is 54 = 54 = 54 with the only diff
being the callee's name; at codegen-units 1 and 16. **Price, stated:** `a..b` syntax does NOT
build it — construction is `Range::from(a..b)` or `Range { start, end }` — so the type is louder
at every construction site than `ops::Range`; and like `ops::Range` it does NOT refuse a
reversed pair (EV-74's honest note stands). Guarantee level: language (the derive and the impls
are in `core`). **Convert LATER:** `crates/boyko_physics/src/solver/colored.rs` — the three
`span: (usize, usize)` parameters of `solve_color`, `solve_color_dispatch` and the group-chunk
dispatcher, and every `span.0` / `span.1` / `let (start, end) = span` read (the tree's only
tuple-span signatures, FV-14). `boyko_physics` is dirty under the KE16 workflow's edits; the
conversion is theirs or the next pass's, not this one's. ERG-02's `Range<T>` clause should name
`core::range::Range` as the spelling for a span that is re-read after being passed.

**FR-02 — `<[T]>::as_chunks::<N>()` for `chunks_exact(N)` followed by a fallible width conversion.**
Stable 1.88 (`#[stable(feature = "slice_as_chunks", since = "1.88.0")]`, FV-09); compiles on
1.97.1. Site: `crates/boyko_utils/src/type_intern/mod.rs`, `impl Hasher for KeyHasher`,
`write` — `bytes.chunks_exact(8)` then `u64::from_ne_bytes(c.try_into().expect("invariant:
chunks_exact(8) yields 8 bytes"))`. `as_chunks::<8>()` yields `(&[[u8; 8]], &[u8])`: the width
IS the type, so the `TryInto`, the `expect` and its string all vanish at source — an ERG-26
"invariant:" that no longer needs stating because nothing can violate it. The same shape in the
index form — `for tri in indices.chunks_exact(3) { tri[0], tri[1], tri[2] }` — is at
`crates/boyko_render/src/tangent.rs` (`compute_tangents`' triangle loop) and
`crates/boyko_render/src/loaders/obj.rs` (`triangle_set`), where `for &[a, b, c] in tris`
replaces three indexings with one irrefutable pattern; the 64-byte-block-plus-remainder shape is
at `crates/boyko_render/src/vg_census.rs` and `crates/boyko_render/src/smaa_luts.rs`. All are
boot / load / tools layer per the ergonomics guide's table, so this is clarity, not speed.
**Cost:** ZERO-COST (A, FV-09): the hasher's `write` is 80 instructions as written and 74 with
`as_chunks`, with ZERO panic calls in BOTH — the optimiser had already proved the `expect` away,
so the type carries what the machine code already knew; the tangent shape is 54 = 54 with 12
diff lines of loop-counter form and 0 `panic_bounds_check` in both; the `try_into` triangle
shape 183 vs 180 with vector ops 111 = 111. At codegen-units 1 and 16. (The instruction deltas
are NOT a cost claim — the ledger's reading rule — the verdict is the panic-call count and the
diff.) Guarantee level: `core` API; the two-tuple return shape is documented. **Convert LATER:**
the five sites above by file and symbol; none is in a crate under edit, but this pass writes no
production code. Zero uses of `as_chunks` exist in the tree today (FV-14), so the first
conversion sets the spelling.

**FR-03 — `#![recursion_limit = "256"]` on `boyko_demo`, to buy off a scheduled hard error today.**
Stable, any version. Site: `crates/boyko_demo/src/lib.rs` (crate root; the struct is
`RenderResources` in `crates/boyko_demo/src/render/mod.rs`, the obligation is at the
`App::new` / `egui` hand-off in `crates/boyko_demo/src/app.rs`). **Buys:** the one finding the
next-generation solver produced on this tree — `overflow evaluating the requirement
RenderResources: Send` / `: Sync` under `recursion_depth_exceeding_limit` (rust-lang/rust#159228,
"this was previously accepted by the compiler but is being phased out; it will become a hard
error in a future release") — disappears before it becomes a break. The depth is wgpu's (every
field is a `wgpu::RenderPipeline` / `Buffer` / `BindGroup`), so this is a limit, not a design
defect. **Cost:** ZERO-COST, COMPILE-TIME ONLY (B, FV-11): `-Znext-solver=globally
-Zcrate-attr=recursion_limit="256"` on `cargo check -p boyko_demo --all-targets` → `EXIT=0`,
`grep -c 'overflow evaluating'` = 0; the attribute changes nothing about what is compiled.
Guarantee level: measured on this tree; the default limit (128) and the attribute are
Reference-level. **Convert LATER:** one line at the top of `crates/boyko_demo/src/lib.rs` with a
comment naming #159228 and the `wgpu` origin; `boyko_demo` is not under edit, but this pass
writes no production code.

**FR-04 — A second, NON-BLOCKING CI leg running the workspace under the next-generation solver, on a nightly pinned by date.**
Nightly only (`-Z` is refused on stable: "the option `Z` is only accepted on the nightly
compiler", FV-10). Site: `.github/workflows/ci.yml`, beside the Miri job. **Buys:** the class of
finding FR-03 is an instance of — a future hard error invisible to the default build — surfaced
months early, plus a standing answer to "does the tree compile under the solver that is about
to be stable" on the day that matters. **Cost:** the leg is ~5 minutes of CI (FV-11 run
receipts, under load: 265–316 s cold) and NOTHING else, on three conditions that are the entire
design. (1) `continue-on-error: true` and NO consumer of its result — so the 7-of-72 fixture
churn (FV-11) is never re-blessed and nothing on stable moves. (2) `RUSTFLAGS="-C
target-cpu=x86-64-v3 -Znext-solver=globally"` — the ISA carried explicitly, because the
variable replaces `.cargo/config.toml` (Method). (3) The toolchain is `nightly-2026-08-20` or
later BY DATE — the installed nightly, on which the whole workspace checks green (FV-11, two
cold runs). ⚠ That nightly is ONE DAY before the solver became nightly's default (announcement
2026-08-21, opt-out `-Znext-solver=coherence`): a pin moved past that date runs `=globally`
whether or not the flag is passed, which is fine for this leg and is exactly why the leg must
keep passing the flag explicitly — so its meaning does not depend on which side of the flip the
pin sits. **Price of the pin:** this is the ONE nightly pin this document endorses adding, and
only because the leg is non-blocking: when the date moves, re-run §5's list, and expect the
`boyko_diag` deprecation (FV-13) to be the first thing it prints. **Convert LATER:** the workflow
file is outside `crates/` but is CI configuration, which this pass does not touch; the leg's
YAML is three steps and the flag string above.

**FR-05 — Drop `-Zmiri-permissive-provenance` from the threadpool's Miri flag string.**
Not a language feature — a Miri flag — recorded here because it is the ONE free move the
strict-provenance probe found, and because the alternative "adoptions" (FR-13) were measured to
buy nothing. Sites: the run recipe in the header of
`crates/boyko_threadpool/tests/miri_scope.rs` (`## Run`), and the campaign's flag line in
`docs/threadpool/KE16-DESIGN-MEASUREMENT.md` (§ Miri). **Buys:** today the pool's Miri gate is
green (FV-12: `running 5 tests` / `3 passed; 0 failed; 2 ignored`) but in a DEGRADED mode —
Tree Borrows, the model `.cargo/config.toml` selects, "does not support integer-to-pointer
casts, so the program is likely to go wrong when this pointer gets used", and every steal goes
through one such cast inside `crossbeam_epoch::Atomic`. The permissive flag SILENCES that
warning; it does not remove the blindness. Without it the run is still green and the three
warnings are visible — all three inside crossbeam-epoch, none in a `crates\` path — so a NEW
int-to-ptr cast introduced in boyko's own code appears as a FOURTH warning naming a `crates\`
path instead of being silenced with the third-party three. That is the difference between a
receipt and a gate. **Cost:** ZERO (B, FV-12): same tests, same `3 passed`, run receipt 8.90 s
vs 10.51 s (under load, not a measurement). **Convert LATER:** `boyko_threadpool` is under
edit by the KE16 workflow and the design-measurement document is that campaign's; both flag
strings are theirs to change, and the header's rationale sentence ("isolates crossbeam's
exposed-provenance int-to-ptr noise … from boyko's own `scope.rs` frames") should be replaced by
the sentence above — the noise is the signal.

---

## §3 REFUSE

Each entry: the technique · why it is tempting · the measured or structural reason · the verdict,
and where a stable spelling exists, the rule that carries it. Ordered by how much the refusal
matters.

**FR-10 — `-Znext-solver=globally` as THE build.** Tempting because the owner named it and
because it accepts the whole tree (FV-11). Refused because it emits no different machine code by
design, its compile-time effect could not be measured on this box, and its ONLY cost is real: 4
of 72 `boyko_ecs` compile-fail fixtures go from one error to seven, printing the engine's own
tailored `on_unimplemented` text three times, and a `.stderr` re-blessed under it is red on the
stable channel — 134 pinned fixtures across 20 suites cannot be green on both solvers at once.
Adopting it as the build converts a coming free stabilisation into a maintained nightly pin
plus a fixture re-bless on every pin move. REFUSED as the build; ADOPTED as the non-blocking
signal (FR-04) and taken when it reaches stable (FR-32). See §1.

**FR-11 — Moving the workspace's build channel to nightly TODAY.** Tempting because the owner
has permitted it and because nightly 1.100.0 (2026-08-20) checks the whole workspace green
(FV-11). Refused because NO feature in this document earns the pin: the two things that would
need nightly (`!` in type position, `f32::maximum`) have measured-free stable stand-ins (FR-30,
FR-31); the solver is arriving on stable on its own (§1); `std::simd` is refused on its merits
(FR-12). The pin's price is concrete and starts on day one: the `-D warnings` clippy gate goes
red over three `fetch_update` → `try_update` deprecations in
`crates/boyko_diag/src/profiling_abi.rs` that stable does not emit (FV-13); a lint the guide
might deny was RENAMED between 1.97.1 and 1.98.0 (FV-12, `fuzzy_provenance_casts` /
`lossy_provenance_casts` → `implicit_provenance_casts`); and `docs.yml`'s own comment records
that newer nightlies broke the tree's layout gates with `E0080`. Each is one fix; each arrives
with every pin move; and today they buy a runtime that is byte-identical. REFUSED until an
entry in §2 or §4 names the feature that pays for it — that is an owner SCOPE decision, listed
as such. The channel stays `stable`; the pinned nightly stays a per-job tool (rustdoc, Miri,
FR-04).

**FR-12 — `std::simd` / `portable_simd` for the bit-exact kernels.** Nightly only (`E0658`,
`portable_simd`, rust-lang/rust#86656, FV-05). Tempting because ONE generic source could serve
the 8-wide AVX2 path and the scalar oracle in `crates/boyko_physics/src/solver/simd.rs`,
`sdf_simd.rs` and `solver/colored.rs` (520 `_mm256_*` sites, FV-14), deleting the
`#[target_feature]` `unsafe` scaffolding. Refused because the conversion is being considered for
exactly one reason — to make the two paths provably agree — and the library's own contract at the
two operations that carry the live failure says they may disagree: `Simd::simd_max` /
`simd_min` lower to `simd_maximum_number_nsz` / `simd_minimum_number_nsz`, whose documentation
(read off the installed nightly's `core/src/intrinsics/simd/mod.rs`) states that when the inputs
compare equal "such as for the case of `+0.0` and `-0.0`, either input may be returned
non-deterministically". That is stated one layer BELOW the public `SimdFloat::simd_max` doc,
which is silent on signed zeros, and the backing intrinsic was renamed and re-specified between
the current stable and this nightly with no change to the public name — the exact "a pin move
drags a silent float-semantics change" hazard. Worse, and independent of `std::simd`: the
engine's own oracle premise is FALSE on the pinned stable. `sdf_simd.rs`'s module doc asserts
that `f32::max`/`min` "return the FIRST operand's sign on a `±0.0` tie"; the standard library
says "either input may be returned non-deterministically" (FV-04, the doc text), and MEASURED
on 1.97.1 (FV-04): at `opt-level=0` `max(+0,-0)` = `-0` and `max(-0,+0)` = `+0` — the SECOND
operand; at `opt-level=3` both orders return `-0`; the non-inlined leaf is `vmaxss` + `vcmpunordss`
+ `vblendvps` with the hardware second-source tie rule. Neither side of the bit-identity gate
is pinned by contract, so no operand ordering can make them agree by contract. REFUSED for
every kernel with a `to_bits()` oracle; the fix for the live red needs no nightly and is FR-31's
stand-in (compare-and-select on BOTH sides). What would change the verdict: a signed-zero
deterministic `simd_maximum` / `simd_minimum` on the vector side (none exists in the 60
intrinsics of the installed nightly) or the tracking issue's still-open "define the semantics
explicitly" item closing with a signed-zero guarantee. It does NOT extend to future SIMD work
with no bit-identity requirement, which is priced separately. The one thing worth copying from
`std::simd` regardless: `simd_clamp` is compare + select, never a `max` instruction — the right
shape for `clamp01_x8`.

**FR-13 — "Adopting strict provenance" in the threadpool, including `expose_provenance` /
`with_exposed_provenance`.** Stable since 1.84 (`#[stable(feature = "strict_provenance", since
= "1.84.0")]`, FV-14). Tempting because the brief said the pool "has no tool-based soundness
evidence" and because the strict-provenance page lists the exposed-provenance pair beside `addr`
/ `map_addr` as if they were the same fix. Refused because there is NO SITE: boyko's own code
already conforms, deliberately — `ACTIVE_POOL` and `WORKER_DEQUE` in
`crates/boyko_threadpool/src/tls.rs` are real pointers, `ScopeShared` is reached through a
`NonNull`, `panic_payload` is an `AtomicPtr`, `component_pool.rs` uses
`ptr::without_provenance_mut`, and `observers/entity_store.rs` documents keeping a fn-pointer
"NOT as a `usize`" for this exact reason; the single int-to-ptr cast in the pool's source is
inside `#[cfg(test)]`. The ONLY blocker under `-Zmiri-strict-provenance` is third-party:
`crossbeam_epoch::Atomic<T>` parks its pointer in an `AtomicUsize` (FV-12). And the tempting
spelling buys nothing: `expose_provenance` / `with_exposed_provenance` emit the SAME Miri
warnings in permissive mode, the SAME hard error under strict mode (whose text names
`with_exposed_provenance` explicitly), and at codegen are LITERALLY THE SAME SYMBOL as the `as`
cast — ICF alias `probe_exposed = probe_casts` (FV-12). The only spelling that satisfies strict
Miri is `AtomicPtr` + `map_addr`, which is free (byte-identical probe pairs) — but it is a
change to crossbeam-epoch (49 lines in one file, semver-breaking on its public `Pointer` trait,
MSRV 1.61 → 1.91), i.e. an in-house-deque decision for the architect, not a Rust-version
adoption. REFUSED as a feature; FR-05 is the free move; the deque question is recorded in §6.

**FR-14 — `#![deny(implicit_provenance_casts)]` (or the older `fuzzy_` / `lossy_provenance_casts`)
on the pinned stable.** Tempting as a mechanical gate for FR-13's discipline. Refused because on
stable it is a gate that CANNOT FAIL: on 1.97.1 and 1.98.0 `#![deny(implicit_provenance_casts)]`
produces `warning: unknown lint` and the build SUCCEEDS with the casts present (FV-12); the lint
is real only on nightly with `#![feature(strict_provenance_lints)]`, and even there it is
crate-local, so it would never see crossbeam. It was also renamed between 1.97.1 and 1.98.0,
so a `deny` written against one name is silently unknown on the next. REFUSED on stable;
BLOCKED-ON-STABLE as FR-33 with its stand-in.

**FR-15 — `f32::algebraic_mul` / `algebraic_add` / `algebraic_sub` / `algebraic_div` / `algebraic_rem`
(stable 1.98).** Gated on the pinned 1.97.1 (`E0658`, `float_algebraic`, rust-lang/rust#136469)
and stable on the side-by-side 1.98.0 (`#[stable(feature = "float_algebraic", since =
"1.98.0")]`) — FV-06. Tempting because it is 1.98's headline float API. Refused everywhere a
scalar oracle and a `to_bits()` gate exist — `solver/simd.rs`, `sdf_simd.rs`, `solver/colored.rs`,
and the scalar oracles in `boyko_math` / `boyko_sdf_math` / `boyko_shaderdsl::field` that ARE
the reference — because it is a per-operation fast-math opt-in, and MEASURED (FV-06, 1.98.0,
`-C target-cpu=x86-64-v3`): `a.algebraic_mul(b).algebraic_add(c)` compiles to exactly ONE
`vfmadd213ss`, byte-identical to `a.mul_add(b, c)`; `a * b + c` stays `vmulss` + `vaddss`; a
slice loop gains 7 `vfmadd` where the plain loop has 0. **This is the highest-value refusal in
the document because the engine's gate does not see it.** The two source censuses
(`solver_simd_has_no_fma_or_approx_callsites`, `sdf_simd_has_no_fma_or_approx_callsites`) forbid
a closed needle set — eight intrinsic stems × two widths + the literal `mul_add(` — and each
scans ONE file. `algebraic_mul(` matches no needle, and a call in the oracle files is outside
the scan. The `.cargo/config.toml` comment ("contraction requires an explicit `f32::mul_add` or
a global fast-math flag") stays TRUE under 1.98 — `algebraic_*` is explicit; it is the CENSUS
that under-covers. REFUSED; and BEFORE the channel is ever moved to 1.98, both censuses gain
`algebraic_` as a stem and a scan over the oracle files (§6, defect 1). A hot loop with no
bit-identity requirement may use it only with a measurement line at the site (ERG-28's
`#[inline(always)]` discipline applies).

**FR-16 — `gen` blocks / iterator generators (nightly, `gen_blocks`, rust-lang/rust#117078).**
Tempting because a generator body reads better than a hand-written `Iterator` state machine.
Refused because the feature's OWN unresolved question is the property ERG-36 makes a MUST: the
tracking issue still lists "whether we should opportunistically implement `size_hint`, and
whether we should implement other traits such as `DoubleEndedIterator` or `ExactSizeIterator`",
and EV-46 measured what a `(0, None)` hint costs on this checkout (1 alloc + 10 reallocs vs
1 + 0 at n = 4096). A generator that cannot state its length is a rule violation with a number
attached. REFUSED; `impl Iterator` (ERG-12) with an honest `size_hint` (ERG-36) is the spelling.
What would change the verdict: `ExactSizeIterator` on `gen` blocks resolved in the affirmative,
and a stable release.

**FR-17 — `allocator_api` (nightly, rust-lang/rust#32838).** Tempting for a `Vec<T, A>` over
the `VmReservation`. Refused because there is no site by principle: CLAUDE.md Principle 0
routes durable bulk data to `ComponentPool` columns on `VmReservation`, and a parameterised `Vec`
would be the parallel data system it forbids. REFUSED; no stand-in needed.

**FR-18 — `<[T]>::get_disjoint_mut` / `get_disjoint_unchecked_mut` (stable 1.86).** Tempting as
the safe two-index borrow. Refused because the tree contains ONE `split_at_mut` site
(`crates/boyko_fontbake/src/msdf/distance.rs`, FV-14) and disjointness in this engine is
archetype-and-`row_ptr` shaped, not slice-index-pair shaped; EV-70 already put `split_at_mut`
and raw pointers at parity, so ERG-24 rests on review budget and there is no pressure this API
relieves. REFUSED for want of a site; permitted where one appears, under ERG-24.

**FR-19 — `Atomic<T>::from_mut` / `from_mut_slice` / `get_mut_slice` (stable 1.98).** Tempting
for the `Box<[AtomicU64]>` summary in
`crates/boyko_ecs/src/ecs/core/component/enable/enable_store.rs`. Refused because that buffer is
BUILT as atomics and regrown as atomics in a `#[cold]` `&mut self` path; no `&mut [u64]` ever
needs to become `&mut [AtomicU64]` or back. REFUSED; nothing to convert.

**FR-20 — `cfg_select!` (stable 1.95).** Zero sites: no `cfg_if` anywhere (FV-14); the tree's
selection idiom is paired `#[cfg]` items, which ERG-33 already governs and which an
expression-position macro does not replace. REFUSED for want of a site.

**FR-21 — `<[T]>::element_offset` (1.94), `subslice_range` / `str::substr_range` (1.98).** Zero
`offset_from` / `sub_ptr` sites (FV-14): the kernel carries `row_ptr` provenance forward and never
recovers an index from a pointer difference. REFUSED for want of a site.

**FR-22 — `hint::select_unpredictable` (1.88) and `hint::cold_path` (1.95).** Plausible and
SITELESS. `cold_path()` is the in-function form of what the tree already has 493 times as
`#[cold]` (FV-14) and EV-60 measured the outlined `#[cold] -> !` helper at 8 vs 18 hot
instructions — the stronger form, and no case was found that cannot be outlined.
`select_unpredictable` chooses `cmov` over a branch; choosing that WITHOUT a measurement at the
site is what Principle 7 and REF-16 forbid, and no site was measured. REFUSED FOR NOW, UNPRICED;
either may be adopted at a named site with an asm diff and a timing beside it.

**FR-23 — AVX-512 (`avx512*` target features and intrinsics, stable 1.89).** Now buildable on
stable for the first time, which makes CLAUDE.md's "AVX-512 optionally via `cfg(target_feature)`"
a real option. Refused AS AN ADOPTION because it is a campaign: a 16-wide kernel bit-identical to
the same scalar oracle (the FR-12 tie hazard included), a third arm in
`tests/isa_baseline_census.rs`, and a downclocking measurement on the target parts. Nothing in
this pass prices it. REFUSED here; an owner SCOPE call to open the track.

**FR-24 — No site: async closures (1.85), RPITIT precise capturing (1.87), naked functions
(1.88), `repr128` (1.89), C-variadics (1.91 / 1.93), `asm_goto` (1.87) / `asm_cfg` (1.93),
`#[diagnostic::do_not_recommend]` (1.85).** The engine has no async surface (0 `async`, FV-14),
no hand-written assembly (0 `asm!`), no 128-bit discriminant, no variadic FFI; return-position
`impl Trait` is already ERG-12's MUST; `do_not_recommend` guards blanket impls the tree refuses
(REF-26). Each is "nothing to express", not "too expensive". REFUSED for want of a site.

**FR-25 — Already adopted; nothing to do.** Recorded so nobody re-proposes them. Safe
`#[target_feature]` functions (1.86) and safe `std::arch` intrinsics (1.87): 18
`#[target_feature(enable = "avx2")]` functions in `solver/simd.rs`, most of them safe `fn`s
calling intrinsics in plain code with the rationale in comments (FV-14) — and the `unsafe` that
remains at the dispatch boundary CANNOT be removed by anything in 1.85–1.98, because the
Reference says a safe `#[target_feature]` function is safely callable only from a caller that
enables the same features, and a `-C target-cpu` flag does not count (its own worked example is
this tree's shape). Strict provenance: `without_provenance_mut` in `component_pool.rs`, `&raw`
as ERG-22's MUST. `offset_of!` (1.77): 334 sites in 34 files. `#[diagnostic::on_unimplemented]`
(1.78): the two kernel marker traits. let-else / let-chains: ERG-38. `#[expect]`: ERG-41.
`MaybeUninit`'s representation and validity (documented 1.92) and the `ManuallyDrop` guarantee
(1.98): assumptions the tree's `UnsafeCell<[MaybeUninit<u8>; N]>` lanes already rest on — the
SAFETY comments may now cite a guarantee instead of folklore. Nothing to adopt.

---

## §4 BLOCKED-ON-STABLE — what we take the day it stabilises, and what to write meanwhile

Each entry: the thing we are waiting for · the tracking issue and what to watch · the STABLE
STAND-IN, measured · what converts, and what breaks, when the gate opens.

**FR-30 — The never type `!` in a type-argument position (`Result<T, !>`).**
GATED on 1.97.1 AND on 1.98.0 (FV-01: `error[E0658]: the `!` type is experimental`,
rust-lang/rust#35121); compiles on the installed nightly with `#![feature(never_type)]`. Watch:
#35121's remaining steps — the edition-2024 fallback change has shipped; full stabilisation is
the later step with no date. **Stable stand-in, measured free:** `core::convert::Infallible`
(FV-02) — `size_of::<Result<u32, Infallible>>() == 4`, the same as a bare `u32`; `Result<(),
Infallible>` is 0 bytes; `Result<u64, _>` 8; `Result<NonNull<u8>, _>` 8; `Option<Infallible>` 0
(all `const _` gates, green); a function returning `Result<u32, Infallible>` unwrapped with `let
Ok(v) = f(x);` is the ICF alias `r_inf = r_bare` of the bare-`u32` function; and that
irrefutable `let Ok(v)` COMPILES on 1.97.1 — matching on an empty enum is exhaustive on stable.
**Conversion when it lands:** none required — `core::convert::Infallible`'s own doc states "When
`!` is stabilized, we plan to make `Infallible` a type alias to it", so a stand-in written today
becomes the real thing without an edit. **What breaks:** the `Infallible` name stays valid by
that plan; the only churn is stylistic. Site today: none (FV-14: 0 uses of `Infallible`); the
shape to use it for is a `Load` / `TryFrom` whose failure is unrepresentable — write
`Infallible`, not a `bool` (ERG-28) and not a `()` error.

**FR-31 — `f32::maximum` / `f32::minimum` — signed-zero-deterministic max and min.**
GATED on 1.97.1 (FV-03: `E0658`, `float_minimum_maximum`, rust-lang/rust#91079); on nightly
with the feature, `maximum(-0.0, +0.0)` and `maximum(+0.0, -0.0)` BOTH return `+0.0` (FV-03) —
"fully deterministic for non-NaN inputs" per its doc. Watch: #91079. **Stable stand-in, measured
free and CHEAPER:** an explicit comparison and select — `if a > b { a } else { b }` — compiles on
1.97.1 at `-C target-cpu=x86-64-v3` to a single `vmaxss %xmm1, %xmm0, %xmm0` (FV-04), while
`a.max(b)` is THREE instructions (`vmaxss` + `vcmpunordss` + `vblendvps`) whose tie result is
unspecified; on a tie the `if` form returns `b` by LANGUAGE SEMANTICS, on every opt level, and
its SIMD twin is `_mm256_cmp_ps(a, b, _CMP_GT_OQ)` + `_mm256_blendv_ps(b, a, mask)` — both
already used in `solver/simd.rs`. That pair pins the zero sign on BOTH sides of the
`to_bits()` gate by construction, which no operand-swap discipline can (FR-12, §6 defect 2).
**Conversion when it lands:** the scalar oracles may spell `maximum` / `minimum` for legibility;
the compare-and-select SIMD side stays, because `f32::maximum`'s vector twin does not exist
(FR-12). **What breaks:** nothing — the stand-in is deterministic today and stays so.

**FR-32 — The next-generation trait solver (`-Znext-solver=globally`) on the stable channel.**
Nightly only today (FV-10); on by default on nightly since 2026-08-21; "plan to stabilize it in
the next months" (rust-lang/rust#107374). Watch: the release notes of the next stable
releases; the day it lands is the day the 134 `.stderr` fixtures are re-blessed ONCE under the
new solver (expect the 7 named in FV-11 to change, 4 of them from one error to seven), the
same day the channel moves — never before, never on a nightly. **Stable stand-in:** the
coherence half is already the default on 1.97.1 (FV-10, §1); the global half is watched by the
non-blocking leg FR-04. **What breaks:** the `RenderResources` overflow (FR-03) if it has not
been taken by then; the seven fixtures above; anything the leg has been printing in the
meantime — which is the point of the leg.

**FR-33 — `implicit_provenance_casts` as a real lint (`strict_provenance_lints`, nightly).**
On the pinned stable the lint is unknown and a `deny` of it is a gate that cannot fail (FR-14,
FV-12). Watch: the stabilisation of `strict_provenance_lints`; note the lint was already renamed
once between 1.97.1 and 1.98.0, so the name to `deny` is whatever the stabilising release
prints. **Stable stand-in:** Miri in warning-visible mode (FR-05) — it sees the casts the lint
would flag, across crates rather than crate-locally, and the pool's run is green under it today.
**What converts when it lands:** `#![deny(implicit_provenance_casts)]` in the kernel crates'
lint tables, expecting ZERO hits in production code (FV-12: the pool's one int-to-ptr cast is
`#[cfg(test)]`) — and, being crate-local, still nothing about crossbeam.

---

## §5 The pin — what it costs, and what to re-check on the day it moves

**The condition, once more, and the two violations.** A nightly is pinned to a date in
`rust-toolchain.toml` and moved deliberately. This tree today: `.github/workflows/docs.yml`
pins `nightly-2026-05-20` (with a comment explaining why: newer nightlies broke the tree's
layout gates with `E0080`); `.github/workflows/ci.yml`'s Miri job uses
`dtolnay/rust-toolchain@nightly` (FLOATING — the UB gate can go red on an upstream change with
no commit in this tree); `crates/bench_bevy_vs_boyko/rust-toolchain.toml` says `channel =
"nightly"` (FLOATING — and that crate is a workspace member, so it can capture any invocation run
from its directory). There is no root `rust-toolchain.toml` (FV-15). Pinning those two is a
higher-value chore than any adoption in §2, and FR-04's leg must not become a third float.

**The re-check list — run it on the day ANY nightly pin moves, in this order, and expect each to
fire at least once in the life of the pin:**

1. `cargo clippy --workspace --all-targets -- -D warnings` on the new nightly — deprecations
   land here first (FV-13: `fetch_update` → `try_update`, three sites in `boyko_diag`, present
   on nightly, absent on stable).
2. `cargo test --workspace --all-targets --no-fail-fast` — the 134 `.stderr` fixtures across 20
   trybuild suites are compiler-version-sensitive (FV-11: 7 of 72 change under the solver
   alone); the ergonomics guide's own history has one that sat red for 87 commits.
3. Every layout gate (`const _: () = assert!(size_of …)`) — `docs.yml`'s comment records `E0080`
   from niche/layout changes on newer nightlies; the ergonomics guide has 38 of them and the
   tree hundreds.
4. `tests/isa_baseline_census.rs` and the two FMA censuses — a pin move that changes how
   `RUSTFLAGS` is assembled in CI can drop `-C target-cpu=x86-64-v3` silently; the census fails
   loudly, which is its job.
5. Lint NAMES in any `deny` / `expect` table — FV-12 recorded a rename across one release.
6. Whether the pin crossed 2026-08-21 — after it, `-Znext-solver=globally` is the default and the
   `RenderResources` overflow (FR-03) is a warning in the DEFAULT build, not only in FR-04's leg.
7. The Miri flag string — `MIRIFLAGS` REPLACES the `[env]` default, so a new Miri can add or
   rename a flag; echo the string and read it.

**What a pin does NOT change.** The machine code of a program both compilers accept: the solver
selects the same impls; none of the features in this document alters codegen except where the
entry says so (FR-15's `vfmadd` is the one that does, and it is refused).

---

## §6 Defects found in passing — not adoption questions, and more valuable than any adoption here

These are for the orchestrator and `docs/OPEN-QUESTIONS.md`; this document records them because
the pass that found them was told to write nothing else.

1. **The FMA census does not see 1.98.** `solver_simd_has_no_fma_or_approx_callsites` and
   `sdf_simd_has_no_fma_or_approx_callsites` forbid eight intrinsic stems × two widths plus
   `mul_add(`, and each scans one file. `f32::algebraic_mul(..).algebraic_add(..)` is ONE
   `vfmadd213ss` on 1.98.0 (FV-06), matches no needle, and can be written in the oracle files the
   censuses never read. Fix before any move to 1.98: add `algebraic_` to the stems; scan
   `boyko_math`, `boyko_sdf_math` and `boyko_shaderdsl::field` as well. Not a Rust question — a
   gate that was written for the language of 1.97.
2. **The `±0` tie doctrine in `sdf_simd.rs` is false on the pinned toolchain.** The module doc
   states that `f32::max`/`min` return the FIRST operand on a tie and builds an operand-swap
   discipline on it; the standard library says "either input may be returned
   non-deterministically", and FV-04 measured the SECOND operand at `-O0` and `-0.0` in both orders
   at `-O3`. The deferred-red `x8_bits_eq_scalar_bits_widened_proptest` says "one of two things is
   true and neither has been established"; there is a third — both sides are unspecified at the
   tie. The fix is stable and cheaper (FR-31's stand-in): compare-and-select on both sides of
   every `max`/`min` in the bit-exact fold. The exact site of the lane-6 divergence is NOT
   localised by this pass (the ignored test was not run).
3. **`docs/rust-ergonomics/EVIDENCE.md`'s Method paragraph is stale on the ISA.** It states that
   no `.cargo/config.toml` sets `target-cpu` and that SSE2 is the shipped baseline; on this branch
   `.cargo/config.toml` sets `-C target-cpu=x86-64-v3` for all three x86_64 targets (with an
   owner-dated rationale). Rows EV-01 … EV-76 were taken at SSE2; the rows in §7 were taken at
   x86-64-v3; the identity results are ISA-independent, the absolute figures are not. Reported,
   not fixed — this pass edits nothing in that file.
4. **CLAUDE.md describes an in-house deque the pool does not have.** "a Chase-Lev work-stealing
   threadpool" — the deque is `crossbeam-deque 0.8` (`crates/boyko_threadpool/Cargo.toml`, and
   `src/lib.rs` says so accurately), and it is precisely where the pool's Miri blindness lives
   (FR-05, FR-13). The two facts an in-house-deque decision needs are measured (FV-12): the
   strict spelling is free at codegen, and the equivalent third-party conversion was 49 lines in
   one file. An architect's call, not a frontier one.
5. **Two floating nightlies** (§5). The mechanism the brief warns about is live today on the UB
   gate, independently of any feature adoption.

Also observed, operational: the C: volume hit 0 bytes free twice during the probes (scratch
target dirs, ~10 GB), and the symptom was `dlltool.exe: CreateProcess` — a mingw-shaped failure,
not a disk message. The repository already records this pattern ("disk to zero masquerades as
mingw", 2026-07-23 audit). Any build on this box that fails strangely should check `df` first.

---

## §7 Evidence — the frontier ledger

This ledger follows the house rules of [rust-ergonomics/EVIDENCE.md](rust-ergonomics/EVIDENCE.md)
and is kept separate from it deliberately: that ledger holds stable-toolchain layout / asm /
allocation / behaviour facts at the SSE2 baseline; this one holds version-gate and nightly
compile-outcome facts at `x86-64-v3`, and its own reading rule — a timing appears only as a run
receipt, labelled under-load, and decides nothing — is stricter. Rows FV-11, FV-12 and FV-13 are
the four probes' measurements, taken today on this box in scratch target directories and cited
with their verbatim `running N tests` / `test result` lines; the rest were produced by the verdict
pass. Nothing under `crates/` was modified.

**FV-01 — `!` in a type-argument position.** (V) `fn parse() -> Result<u32, !>`: 1.97.1 →
`error[E0658]: the `!` type is experimental … see issue #35121`; 1.98.0 → the same `E0658`;
nightly 2026-08-20 with `#![feature(never_type)]` → `EXIT=0`, prints `7`. → GATED on both stables.

**FV-02 — `core::convert::Infallible` as the never type's stable stand-in.** (L, A, B) Gates,
all green on 1.97.1: `size_of::<Result<u32, Infallible>>() == 4`; `Result<(), Infallible>` 0;
`Option<Infallible>` 0; `Result<u64, Infallible>` 8; `Result<NonNull<u8>, Infallible>` 8.
`r_inf` (`let Ok(v) = parse_x(x); v` over a `#[inline(never)]` `-> Result<u32, Infallible>`) vs
`r_bare` (the bare `-> u32` twin): `-O -C target-cpu=x86-64-v3 --emit=asm` prints the ICF
alias line `r_inf = r_bare`. The irrefutable `let Ok(v) = …` compiles on 1.97.1 (exhaustive
matching on the empty enum). `core/src/convert/mod.rs` (installed nightly): "When `!` is
stabilized, we plan to make `Infallible` a type alias to it". → ZERO-COST; converts by alias.

**FV-03 — `f32::maximum` / `minimum`.** (V, B) 1.97.1 → `error[E0658]: use of unstable library
feature `float_minimum_maximum` … issue #91079`. Nightly with the feature, `-O -C
target-cpu=x86-64-v3`: `maximum(-0,+0) = 0x0`, `maximum(+0,-0) = 0x0` (both `+0.0`). → GATED;
deterministic where available.

**FV-04 — `f32::max` on a `±0` tie, and the compare-and-select stand-in, rustc 1.97.1.** (A, B)
Runtime, operands through `black_box`: `opt-level=0`, `x86-64-v3`: `max(+0,-0) = 0x80000000`
(`-0`), `max(-0,+0) = 0x0` (`+0`), `min(+0,-0) = 0x80000000`, `min(-0,+0) = 0x0` — the SECOND
operand in every case; `clamp(-0, 0, 1) = 0x80000000`. `opt-level=3`, `x86-64-v3` AND the
SSE2 baseline: `max(+0,-0) = max(-0,+0) = 0x80000000`, `min(+0,-0) = min(-0,+0) = 0x80000000`,
`clamp(-0, 0, 1) = 0x80000000` — `-0` in BOTH orders. Asm, `-O -C target-cpu=x86-64-v3`, a
non-inlined leaf: `a.max(b)` = `vmaxss %xmm1, %xmm0, %xmm1; vcmpunordss %xmm1, %xmm1, %xmm2;
vblendvps %xmm2, %xmm0, %xmm1, %xmm0; retq` (3 instructions; MAXSS returns its SECOND source on
a zero tie); `if a > b { a } else { b }` = `vmaxss %xmm1, %xmm0, %xmm0; retq` (1 instruction;
the tie returns `b` by language semantics). An 8-lane `[f32; 8]` `max` loop: `vmaxps` +
`vcmpunordps` + `vblendvps`. Contract text (installed nightly `core/src/num/f32.rs`, `max`
and `min`): "If the inputs compare equal (such as for the case of `+0.0` and `-0.0`), either
input may be returned non-deterministically." → the FIRST-operand doctrine in
`crates/boyko_physics/src/sdf_simd.rs` does not hold at ANY opt level measured; the explicit
compare is deterministic AND two instructions shorter (the count is not the verdict; the
determinism is).

**FV-05 — `std::simd`.** (V) `use std::simd::f32x8;` on 1.97.1 → `error[E0658]: use of unstable
library feature `portable_simd` … issue #86656`. → GATED.

**FV-06 — `f32::algebraic_*`.** (V, A) 1.97.1 → `error[E0658]: use of unstable library feature
`float_algebraic` … issue #136469`. 1.98.0 (`#[stable(feature = "float_algebraic", since =
"1.98.0")]`), `-O -C target-cpu=x86-64-v3 --emit=asm`, rlib: `alg` (`a.algebraic_mul(b)
.algebraic_add(c)`) = `vfmadd213ss %xmm2, %xmm1, %xmm0; retq`; `fused` (`a.mul_add(b, c)`) =
the identical `vfmadd213ss`; `plain` (`a * b + c`) = `vmulss; vaddss; retq`. `alg_slice`
(the same over slices) contains 7 `vfmadd`; `plain_slice` 0. The censuses' needle set
(`crates/boyko_physics/src/solver/simd.rs`, `solver_simd_has_no_fma_or_approx_callsites`, read
verbatim): stems `["fmadd", "fmsub", "fnmadd", "fnmsub", "fmaddsub", "fmsubadd", "rsqrt",
"rcp"]` × widths `["_mm256_", "_mm_"]` + suffix `"_ps("`, plus `"mul_add("`; one file per census.
→ COSTS the bit-identity contract; invisible to the gate.

**FV-07 — `core::range::Range<usize>`, version and shape.** (L, B, V) `#[stable(feature =
"new_range_api", since = "1.96.0")]`; compiles on 1.97.1. Gates: `size_of::<Range<usize>>() ==
16 == size_of::<core::ops::Range<usize>>()`; `Option<Range<usize>>` 24; `Range<u32>` 8;
align 8. `is_copy::<core::range::Range<usize>>()` compiles; `is_copy::<core::ops::Range<usize>>()`
→ `error[E0277]: the trait bound `std::ops::Range<usize>: Copy` is not satisfied`. The re-read
shape `take(span); let _n = span.end - span.start;` over `ops::Range` → `error[E0382]: use of
moved value: `span``. `Range::from(2..5)`, `Range { start, end }`, `xs[r]` (→ `SliceIndex`),
`for i in r`, `r.is_empty()`, `r.contains(&3)`, `let o: ops::Range<usize> = r.into()` all
compile and run. → the friction EV-74 did not price is real and is `Copy`.

**FV-08 — The `colored.rs` span in three spellings.** (A) `solve_t` (`(usize, usize)`,
destructured), `solve_o` (`ops::Range`), `solve_c` (`core::range::Range`): 17 = 17 = 17
instructions, 0 normalised-diff lines pairwise. `dispatch_t` / `dispatch_o` (with `.clone()` at
the first hand-off) / `dispatch_c` (pass down, `slots = end - start`, containment check, pass
again): 54 = 54 = 54, the diff being the two `callq` target names. `solve_cs` (`for &x in
&xs[span]`): 23 instructions, both bounds compares hoisted before the loop, none inside — the
EV-74 shape reproduced at `x86-64-v3`. Held at `codegen-units = 1` and at the shipped 16
(`--out-dir`). → ZERO-COST.

**FV-09 — `as_chunks::<N>` vs `chunks_exact(N)` + conversion, three site shapes.** (A, V)
`#[stable(feature = "slice_as_chunks", since = "1.88.0")]`; compiles on 1.97.1. `write_c`
(`chunks_exact(8)` + `try_into().expect(..)` + remainder, the `KeyHasher::write` shape) 80
instructions vs `write_r` (`as_chunks::<8>()`) 74; panic / `expect_failed` calls 0 in BOTH;
vector ops 0 / 0. `tri_index_c` (`chunks_exact(3)` + `tri[0], tri[1], tri[2]`, the `tangent.rs`
shape) vs `tri_index_r` (`for &[a, b, c] in tris`): 54 = 54, 12 diff lines of loop-counter
form, `panic_bounds_check` 0 in both, 3 calls each. `tri_c` (`chunks_exact(3)` + `try_into`) vs
`tri_r`: 183 vs 180, vector ops 111 = 111. Held at 1 and 16 units. → ZERO-COST; the deletion is
at source (the `TryInto`, the `expect` and its string).

**FV-10 — The `-Znext-solver` option table on every installed compiler.** (V)
`rustc -Znext-solver=bogus --emit=metadata`: 1.97.1 and 1.98.0 (with `RUSTC_BOOTSTRAP=1` set
SOLELY so the stable binary prints its option table; never used for a build) and nightly
2026-08-20 all print the identical `error: incorrect value `bogus` for unstable option
`next-solver` - either `globally` (when used without an argument), `coherence` (default) or
`no` was expected`. Without the bootstrap variable, 1.97.1: `error: the option `Z` is only
accepted on the nightly compiler`. Release notes 1.84.0: "Use the next-generation trait solver
in coherence" (rust-lang/rust#130654). → coherence is the compiled-in default on the pinned
stable; `globally` is nightly-only; this upgrades the solver probe's inference to a reading.

**FV-11 — The next-solver probe's workspace measurements (cited).** (B, S; run receipts under
load) Nightly 2026-08-20, `RUSTFLAGS="-C target-cpu=x86-64-v3"`, scratch `CARGO_TARGET_DIR`,
`cargo check --workspace --all-targets`: `EXIT=0`, 0 errors, two cold runs (receipts 265 s and
299 s — the same configuration, 13 % apart, which is the noise floor). `-Znext-solver=globally`:
`EXIT=0`, 0 errors; the warning-set diff against baseline is exactly `overflow evaluating the
requirement RenderResources: Send` and `: Sync` (`crates/boyko_demo/src/app.rs`), lint
`recursion_depth_exceeding_limit`, rust-lang/rust#159228 (receipt 316 s). `-Znext-solver=no`:
`EXIT=0`, 0 errors (receipt 168 s). `-Zcrate-attr=recursion_limit="256"` with `=globally` on
`cargo check -p boyko_demo --all-targets`: `EXIT=0`, `grep -c 'overflow evaluating'` = 0. Flag
delivery proved: `=bogus` in the same `RUSTFLAGS` fails the build at the first rustc. 72
`boyko_ecs` compile-fail fixtures replayed per-file with `cargo check` in a scratch crate under
both configurations: identical 64, harness-path artefact 1, span shift 3 (relations fixtures,
`required by this bound` col 18 → 37), 1 → 7 errors 4
(`compile_fail_chunk__mut_data_rejected`, `compile_fail_chunk__ref_data_rejected`,
`option_anyof_compile_fail__for_each_chunk_rejects_anyof`, `…_rejects_option`; each gains
`E0271` and prints the `ChunkedQueryData` `on_unimplemented` text three times); flipped to
accept 0. `find crates -name '*.stderr' | wc -l` = 134 over 20 directories. Compile time: NOT
MEASURABLE — an interleaved paired `cargo clean -p boyko-ecs` / `check` experiment was slower
under `=globally` in 3 of 3 early pairs and faster in 2 of the next 3 as load changed.
→ accepts the tree; one future hard error found; diagnostics churn measured; no timing.

**FV-12 — The strict-provenance probe's measurements (cited).** (B, A, L, V; receipts under
load) `cargo +nightly miri test -p boyko-threadpool --test miri_scope` with
`MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-permissive-provenance
-Zmiri-ignore-leaks"`: `EXIT=0`, `running 5 tests` / `test result: ok. 3 passed; 0 failed; 2
ignored; 0 measured; 0 filtered out; finished in 10.51s` (the two ignores are KE16 feature-arm
deferrals). The same WITHOUT `-Zmiri-permissive-provenance`: `EXIT=0`, `3 passed`, plus three
`warning: integer-to-pointer cast` — all in `crossbeam-epoch-0.9.20/src/atomic.rs` — including
"Tree Borrows does not support integer-to-pointer casts, so the program is likely to go wrong
when this pointer gets used" (receipt 8.90 s). With `-Zmiri-strict-provenance`: `EXIT=1`,
`error: unsupported operation: integer-to-pointer casts and `ptr::with_exposed_provenance` are
not supported with `-Zmiri-strict-provenance`` at `crossbeam-epoch-0.9.20/src/atomic.rs`
(`unsafe fn deref(ptr: usize)`), reached from `crossbeam_deque::Stealer::steal_batch_and_pop`
← `boyko_threadpool::worker::try_steal_random` ← `worker_main`. boyko-owned int→ptr casts in
production code: 0 (`tls.rs` holds `Cell<*const PoolInner>` and a pointer pair; `scope.rs`
holds `NonNull<ScopeShared>` and `AtomicPtr<Box<dyn Any + Send>>`; the only cast is in
`tls.rs`'s `#[cfg(test)]` module). Three spellings of crossbeam-epoch's tagged `Atomic<T>` in
a scratch crate: `as`-cast and `expose_provenance` / `with_exposed_provenance` produce the SAME
two Miri warnings and the SAME strict-mode error; `AtomicPtr` + `map_addr` passes strict mode
with zero warnings. `rustc -O -C target-cpu=x86-64-v3 --emit=asm`: ICF alias `probe_exposed =
probe_casts`; `probe_casts_len` / `probe_strict_len` byte-identical (4 instructions),
`_tag` (3) and `_settag` (6) likewise; layout gates `size_of::<AtomicUsize>() ==
size_of::<AtomicPtr<Bag>>()`. crossbeam-epoch converted outside the repo: 49 lines added, 49
removed, 24 hunks, ONE file of 8 (`atomic.rs`); crossbeam-deque 0.8.7 compiles unchanged
against it; a two-thread `steal_batch_and_pop` test under strict mode: unpatched `EXIT=1` at
`atomic.rs`, patched `running 1 test` / `test result: ok. 1 passed; 0 failed` (receipt 2.99 s).
Release asm at the real call sites against both epochs: `bench_push` byte-identical (25),
`bench_pop` 8 = 8, `bench_steal` 347 → 351 with `callq` 6 = 6, `lock` 5 = 5, fences 0 = 0 (the
+4 is `with_addr`'s `wrapping_sub` / `wrapping_byte_offset` lowering, which the stdlib's own
comment calls unwanted arithmetic). `Pointer<T>` is public API (`into_usize` / `from_usize`
change) and the conversion needs `AtomicPtr::fetch_or` (1.91) against an MSRV of 1.61. Lints:
`rustc -W help` lists `fuzzy-provenance-casts` + `lossy-provenance-casts` on 1.97.1 and a single
`implicit-provenance-casts` on 1.98.0 and nightly; `#![deny(implicit_provenance_casts)]` on
1.97.1 or 1.98.0 → `warning: unknown lint`, build SUCCEEDS with the casts present; nightly +
`#![feature(strict_provenance_lints)]` → two hard errors. NOT run: the pool's own `miri_scope`
against the converted epoch (the closing command is recorded in the probe), and a workspace
lint sweep (died at `No space left on device`). → nothing to adopt; the permissive flag is the
free move; the deque is the lever.

**FV-13 — Nightly-only deprecation under `-D warnings`.** (V) `cargo check -p boyko-diag` piped
to `grep -c deprecated`: stable 1.97.1 → 0; nightly 2026-08-20 → 3 —
`Atomic::<u64>::fetch_update` deprecated in favour of `try_update`, site
`crates/boyko_diag/src/profiling_abi.rs` (the `ARM_MASK.bits.fetch_update(Release, Acquire, …)`
projection). → a day-one red for any nightly build channel.

**FV-14 — Structural counts over `D:/wt/threadpool/crates` at `51d9a9bf` + the dirty tree.** (S)
`offset_of!` 334 occurrences in 34 files. `#[target_feature(enable = "avx2")]` in
`solver/simd.rs` 18. `#[cold]` 493. `_mm256_*` production sites 520. `split_at_mut` /
`get_disjoint_mut` outside tests 1 (`boyko_fontbake/src/msdf/distance.rs`). `offset_from` /
`sub_ptr` 0. `cfg_if` 0. `asm!` 0. `async` 0. `as_chunks` 0. `Infallible` 0. `chunks_exact(`
production sites 9, of which 1 with `try_into().expect` (`type_intern/mod.rs`).
`span: (usize, usize)` parameters 3, all in `solver/colored.rs`; `.clone()` on a range 0.
`#[diagnostic::on_unimplemented]` production sites 2 (`chunked_data.rs`, `query/filter.rs`).
`.stderr` fixtures 134. `Box<[AtomicU64]>` 1 (`enable_store.rs`).

**FV-15 — Toolchain pins in the tree.** (S) `find . -maxdepth 3 -name 'rust-toolchain*'` →
only `crates/bench_bevy_vs_boyko/rust-toolchain.toml` (`channel = "nightly"`, floating). No root
`rust-toolchain.toml`. `.github/workflows/ci.yml`: eight `dtolnay/rust-toolchain@stable` steps
and one `dtolnay/rust-toolchain@nightly` (the Miri job, floating). `.github/workflows/docs.yml`:
`toolchain: nightly-2026-05-20` (pinned, with the `E0080` rationale). `.cargo/config.toml`:
`MIRIFLAGS = "-Zmiri-tree-borrows"` and `-C target-cpu=x86-64-v3` per x86_64 target.

---

## Files

- [docs/RUST-FRONTIER.md](RUST-FRONTIER.md) — this document: the trait-solver answer, ADOPT,
  REFUSE, BLOCKED-ON-STABLE, the pin's re-check list, defects found in passing, the frontier ledger.
- [docs/RUST-ERGONOMICS.md](RUST-ERGONOMICS.md) — the rules for code written now on stable; FR-01
  and FR-02 extend ERG-02 and ERG-26 once converted.
- [docs/rust-ergonomics/EVIDENCE.md](rust-ergonomics/EVIDENCE.md) — the stable ledger (EV-01 …
  EV-76, SSE2); its Method paragraph is stale on the ISA (§6 defect 3).
- `docs/threadpool/KE16-DESIGN-MEASUREMENT.md` — owns the Miri flag string FR-05 asks to change.
