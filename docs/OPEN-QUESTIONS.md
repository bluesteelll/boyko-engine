# Open questions for the owner

Difficulties, disputable calls and things I did not understand — written down as they arise so the
owner can read them later and weigh in, rather than finding them buried in a report after the
decision was already made.

> Russian version: [`ru/OPEN-QUESTIONS.md`](ru/OPEN-QUESTIONS.md). **This file is the source of
> truth**; editing either side updates the other in the same commit. See [`ru/README.md`](ru/README.md).

**Convention.** Newest first. Each item states the situation, the options, and what it blocks. An
item is marked `RESOLVED` with the date and the owner's decision rather than deleted — the record of
*why* a call was made outlives the call. Perf and architecture forks are decided without asking, with
numbers; what lands here is VALUES, SCOPE, and anything genuinely unclear.

---

## 2026-08-12 — L8a: two SCOPE calls I made narrowly, and one I did not make at all

Rung L8a migrated `boyko_render` / `boyko_image` / `boyko_serialize` / `boyko_physics` (16 sites,
twelve codes). Three things sat on the boundary between "decide it yourself with numbers" and "this
is the owner's". I decided two and am recording them for review; the third I am not deciding.

**1. `resolve_candidate` conflates an ABSENT optional texture with an UNREADABLE one, and I left it
conflated.** `crates/boyko_render/src/texture.rs`'s `load_slot` has two arms: a file that decodes
wrong is now `boyko-W2206` (a `Warn`), and a file that does not resolve is an `info!` with no code,
because all five material-folder slots are documented as optional and warning on absence would put
a `Warn` in the log of every material that ships four maps instead of five.

The problem is that the second arm also covers a file that *exists and cannot be read* — a
permissions fault, a locked file, a bad mount — because `resolve_candidate` discards the
`io::Error` with `.ok()`. So a real fault is reported at `info` level, indistinguishable from a map
somebody simply chose not to author.

**The cost of splitting it**, measured: `resolve_candidate` returns
`Option<(PathBuf, Vec<u8>)>` and would have to return the reason, which means re-blessing the three
tests that pin its `Option` signature (`resolve_candidate_prefers_the_first_existing_candidate_in_order`,
`..._falls_back_to_a_later_alias_when_the_first_is_absent`, `..._returns_none_when_no_candidate_resolves`).
That is a signature change to a helper in the asset load path, inside a rung whose scope is
"replace `eprintln!` with a coded emitter". I kept the rung's scope and recorded the hole rather
than widening quietly. **If you want it split, it is a small, self-contained change and I will do
it in its own commit.**

**2. `RatePolicy` is declared on every registry row and applied by nothing.** Measured by reading
the expansion, not the design: `warn!`/`error!` gate on the three ceilings and call `emit_impl`;
neither reaches `rate::admit`, which still has zero production callers. Every `Once` in this
registry works because a human placed an `OnceSite` at the emitter.

That is not itself a defect — `Once` and `Every` need no machinery. What it means is that a row
declaring `EveryN` or `MinIntervalMs` would be a **promise with nothing behind it**, and no check
would notice. I added `no_live_row_declares_a_policy_the_emission_path_cannot_honour` to `codes.rs`,
which reds on exactly that. It cannot prove a declared `Once` has an `OnceSite`; its failure text
says so.

**The question is what happens to `rate::admit`.** It has been carried for several rungs with no
caller. Either L11a/L14 wire it into the emission path — which puts a rate check on the enabled
path of every `Warn`/`Error` — or it is deleted and the registry column narrows to the three
policies the engine can actually honour. I have no measurement that favours either, and it is a
scope call.

**3. `E2203` floods, and I did not damp it.** `GpuSystem::run_unsafe` has no `Result` channel, so a
device fault that recurs reaches an operator only as a record per frame. I declared `Every`,
matching the `eprintln!` it replaced, on the reasoning that a `Once` would report the first bad
frame of a session and let an hour of broken frames look identical to a good one. The flood is
bounded by the ring, which drops and counts. If you would rather see one line per second than one
per frame, that needs item 2 resolved first — there is no mechanism today that could deliver it.


## 2026-08-11 — ⚠️ MEASURED: validation DOES run on this box, and the 2026-08-06 entry below is narrower than it reads

Opening logging rung L7 (migrate `boyko_rhi_vulkan`) started by re-deriving the site list, because
the rung's row cites line numbers that have drifted. It also had to establish what `E2101` — "add
an `error!` when validation is requested but the node was not chained" — can actually observe here.
Two measurements, both against HEAD, **before any L7 code was written**:

1. **A validation-ON boot works.** With `BOYKO_DISABLE_VALIDATION` **unset** and
   `enable_validation: true`, `cargo test -p boyko_rhi_vulkan --test compute` boots and passes
   **4 of 4**. The standing note that the SDK's MSVC-built `VkLayer_khronos_validation.dll` crashes
   this MinGW process on load is not true of the **headless compute path**. Whatever it describes —
   most likely the windowed/golden path — it is narrower than "validation cannot run here", and I
   have been treating it as the wider claim.
2. **The chained validation-features node is built, not unbuildable.** `create_instance` enables
   `VK_EXT_validation_features` when present and chains `VkValidationFeaturesEXT` with
   synchronization validation as the head of the instance `p_next`. Disposition **F2** ("a chained
   validation-features node is unbuildable here") is refuted by the tree.

**What that costs the plan.** G7's first clause — "`E2101` fires on a validation-**on** run" — holds
only if the node can never be chained. It can, so on a correct box a validation-on run must be
**silent**, and the gate as specified would be red against a working engine. I have re-cut `E2101`
to mean *validation was requested and this process is not getting it* (the escape hatch took it, or
the extension is absent), which makes G7 two-sided and runnable here: positive = escape hatch set,
negative = unset. The full argument is in the corpus (`logging/ladder`, the L7 block).

**This is an architecture call and I made it** rather than waiting — it is a gate's polarity, not a
value. It is here because it **contradicts a disposition the owner may have relied on**, and because
of what it does NOT change: `M25` stands. `compute.rs`'s own `negative_chained_barrier_hazard`
documents in the tree that sync-validation is enabled and still does **not** flag a compute→compute
RAW hazard on this path. The layer being *present* and the layer being *sensitive* are two
questions; L7 can gate the first and nothing gates the second.

> ⚠️ **A question I asked here and then measured, and it should not have been asked.** The first
> version of this entry offered the owner a choice: every golden runs under
> `BOYKO_DISABLE_VALIDATION=1`, so after L7 each one emits a `boyko-E2101` line — *"should it be
> suppressed for the golden legs?"* **Both halves of the premise are false**, and the question's
> shape was worse than either: it invited weakening a diagnostic to protect a channel, when
> **saying that a golden run's validation was disabled is the entire reason the code exists.**
> Suppressing it there would deliberately rebuild the defect the 2026-08-06 entry below describes.
>
> 1. **No collision is possible.** `scripts/golden.ps1:226` scans with the literal pattern
>    `\[vk-validation\]`. `boyko-E2101` cannot match it.
> 2. **In a golden run the line does not exist at all.** Measured: **no host calls
>    `boyko_log::lifecycle::boot` or `enable`** — the only callers anywhere are `boyko_log`'s own
>    tests and `boyko_ecs/tests/log_seam.rs`, and `crates/boyko_ecs/src/ecs/core/log/plugin.rs:40`
>    says so in its own doc comment. So the `error!` goes into a `.bss` lane ring nothing drains,
>    and not one byte is printed.

### And (2) is the finding that matters more than the question it answers

**The logger is in exactly the state the profiler was in at `e0160555`: complete, gated, and
unreachable from every host.** L5 landed the ECS seam, L6 landed the engine's emitters, and nothing
turns it on — so every record L6 just wired up is written into a ring with no consumer. Twelve
`Live` rows, five new codes, ten doc pages, and in a shipped run the whole apparatus is silent for a
reason no gate reports.

It is not a defect *of* L5 or L6 — `boot`/`enable` belong to `boyko_app`, which is **L8b's** row, and
`plugin.rs` was written knowing it. What is worth the owner's attention is that this is the **same
shape, in the same campaign, two rungs after it was found the first time**: every gate builds its own
world, enables logging itself, and asks whether the record arrived — so none of them can see that no
host ever does. I am recording it now rather than at L8b because the last time this shape appeared,
fifteen green rungs had passed over it.

**Nothing is blocked and no decision is needed**; L8b closes it by construction. If the owner wants
it closed *earlier* — a host that boots the logger before L7's migration lands, so L7's own sites are
observable in a real run rather than only in tests — that is a scope call and the only one here.

---

## 2026-08-11 — L6 found three mechanisms that exist and are unreachable, and left them that way on purpose

Logging rung L6 migrated `boyko_ecs` and `boyko_threadpool`. Three things it touched are **built,
correct, and consumed by nobody**. None of them blocks the rung; each is recorded here because
"reached for it and decided not to" is the only thing that distinguishes a deliberate gap from an
oversight, and because two of them are the same shape as the defect L6 opened with.

**1. `TargetControl`'s sync-route bit has no reader.** `target.rs` packs `bit [7] sync route —
format on the caller, write synchronously`, with a constructor, an accessor, a CAS that preserves
it and its own unit tests. `grep sync_route` over `crates/boyko_log/src` returns **`target.rs` and
nothing else**: `emit_impl` never consults it. A target with the bit set behaves exactly like one
without. Its intended writer is `apply_control_spec` (L14, the `net=debug/6!` form), so the bit is
early rather than wrong — but a *control* nobody reads is exactly what `site.decode` was, and that
one went three rungs unnoticed. **Not implemented at L6** because honouring it means a second
emission path (render on the caller, take `OUT_LOCK`) which is L14's row and needs L14's gates.

**2. `rate::admit` has zero production callers.** The rate limiter is complete and unit-tested —
`EveryN`, `MinIntervalMs`, the 512 cache-line slots, the suppressed counter. Every engine registry
row declares `Every` or `Once`, and both are answered by a **site-local latch** by design (F11), so
`RATE` is never touched. L6 considered `MinIntervalMs(1024)` for `W0701` — an event lane that
refuses every frame is exactly what a per-second cap is for — and **refused**: it drags a clock read
onto a cold ECS path and puts the rate decision *ahead* of the macro's own runtime gate, so a
disabled target would pay for a policy on a record it will not emit. The honest statement is that
`RATE`'s 32 KiB of `.bss` is reserved for a policy no engine row currently declares.

**3. `E0201`'s stderr fallback owes a `print_allowlist.txt` row at L8c.** `abort_on_task_panic`
prints for itself **iff** `flush()` answered `NoConsumer` — see the L6 decision block for why the
ledger's `error!` + `flush()` alone would have made the abort decision invisible in a
diagnostics-off process. L8c's `print_census.rs` bans `eprintln!` in production; this site needs a
row naming that reason. Flagged now because L8c is four rungs away and an unexplained allowlist
entry written then would read as laundering.

**What the owner may want to decide**: nothing is blocked. If (1) or (2) should be *deleted* rather
than left for L14/L11a — a bit and an array that cost `.bss` and reader attention — that is a
values call, and it is the opposite of the call this campaign has been making (absent rather than
stubbed). My own reading is that both are fine to keep, because both have a named consumer at a
named rung, which `site.decode` never did.

---

## 2026-08-11 — ⚠️ A profiling test's HAND-PICKED zone id is a bet against every schedule the rest of the crate runs. `ZONE = 7` still is one.

Found by rung 12's `G18`, which is the first profiling gate to assert an **exact session total** rather
than a single cell. In a full-workspace sweep it counted **20 013 of 20 000** samples: thirteen it
never pushed.

**The mechanism, and why the module lock cannot help.** `profiling/tests.rs` serialises every test
that arms, on one `test_serial()` lock — which is correct and insufficient. `ARM_MASK` is
**process-global**, so while any profiling test holds the profiler armed, **every other test in the
`boyko-ecs` binary that runs a schedule emits `SystemSpan` samples** (`profiling/zones.rs:193`) —
on its own thread, into its own lane, which the fold drains along with everything else. Those samples
carry per-system zone ids minted at `try_build` out of the same monotone `ENGINE_ID_NEXT` the static
zones use.

A hand-picked `const ZONE: u16 = 7` is therefore a bet that no system anywhere in the crate's test
suite lands on id 7 — and the bet is **re-rolled by every change to test execution order**, which is
why it had never fired before.

**Fixed for rung 12's own zone:** the tier tests now use a `declare_zone!`-minted handle. The counter
is monotone and shared, so an id that handle owns is one no `SystemMeta` can ever be given. Not a
mitigation — the collision becomes unrepresentable.

**NOT fixed, and this is what the owner may want to decide.** `const ZONE: u16 = 7` at
`profiling/tests.rs:41` is still a raw number, used by roughly thirty assertions across the rung-2
and rung-3 suites. They are far less sensitive — they read one `(frame, zone)` cell after a drain
they control, rather than a session total — so a stray sample would have to land in the same frame
*and* the same cell to be seen at all. But the hazard is the same one, and it is the kind that
surfaces as an inexplicable off-by-N years later.

Two ways to close it, neither urgent:

1. **Mint it too** (`declare_zone!` + a `fn zone()`), mechanical but touches ~30 assertions in tests
   that currently pass — a diff whose risk is entirely in the churn.
2. **Leave it and rely on the insensitivity**, with the hazard recorded here, which is the state
   today.

⚠️ **The general form is worth more than the instance:** *any* test that arms the process-global
profiler is measuring a channel the rest of the test binary is also writing to. A profiling gate that
asserts a total — as every rung-12-and-later gate does — must own an id nothing else can be given.

---

## 2026-08-11 — ⚠️ The `trybuild` corpus is blessed for a DIFFERENT rustc than the toolchain the project mandates. 23 fixtures were red at `3163078f`.

Found while sweeping rung 11, and **proved not to be rung 11's** with `git stash`: with the whole
working tree stashed, `cargo test --no-fail-fast` over the eight `compile_fail` suites at
`3163078f` reds **seven of them, 23 fixtures**, under
`RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-gnu` (rustc **1.97.1**, installed 2026-08-04).

**The diff has TWO causes, and my first diagnosis of it was WRONG — recorded that way because the
wrong one is the plausible one.** I wrote "compiler rendering drift between rustc 1.95 and 1.97.1"
and then tested it. Both claims below are probe results, not inference.

**Cause 1 — RUNG 11'S OWN, and it is not drift at all: `impl_self_bundle!` crossed a rustc
rendering THRESHOLD.** `compile_fail_relations/relationship_hook_collision.stderr` changed from a
span-based `help:` block into an inline `= help:` list of bare names. That looks exactly like a
compiler-version change and is not one. MEASURED with a five-line probe compiled by both binaries:
**rustc 1.95.0 and 1.97.1 render the identical trait-bound error identically**, and **rustc switches
from spans to the compact list at FIVE candidate impls**:

```text
3 impls -> help: … --> probe.rs:2:12  |  2 | struct T1; impl Marker for T1 {}      (spans)
4 impls -> spans
5 impls ->   = help: the following other types implement trait `Marker`:  T1 T2 T3  (list)
```

The `Component` list held **three** entries (`ChildOf`, `Children`, `LikedBy`) and rung 11 added
**three more** (`ProfilingScope`, `ProfilingScopeEnabled`, `ProfiledZone`) — six, past the threshold,
so the whole block re-rendered. `compile_fail_relations` was **green on the clean tree and red with
the change**: that suite is entirely mine. The same two `impl_self_bundle!` lines also appear inside
several other fixtures' `Bundle` lists without flipping their format.

⚠️ **The general lesson, which is worth more than this instance: adding ONE trait impl anywhere in
the engine can re-render every pinned diagnostic that lists that trait's implementors — and past the
fifth impl it changes the FORMAT, not just the contents.** A `.stderr` corpus is coupled to the
engine's impl *count*, invisibly.

**Cause 2 — INHERITED, 23 fixtures across 7 suites, and its origin is UNDETERMINED.** Proved not to
be mine by `git stash`, above. The signatures are additions and substitutions the blessed files do
not carry — e.g. `compile_fail_chunk/mut_data_rejected.stderr` gains an entire
`note: required by a bound in Query::<'w, 's, D, F>::for_each_chunk` block; elsewhere
`\| $crate::panicking::panic_fmt(…)` collapses to `= note: the failure occurred here`, and `AtomicIN`
becomes `Atomic<iN>`.

**I could not determine what produced them, and I am not guessing in this file.** The hypothesis I
had — that rung 10's *"543 targets ok, 0 failed"* was measured under the chocolatey `cargo`/`rustc`
1.95.0 that shadows `~/.cargo/bin` on `PATH` (real, found in this session, and the cause of a phantom
`E0133` on `__cpuid` plus a wall of MSVC `link.exe` failures) — is **not supported** by the probe:
the two compilers agree on the renderings I could test. The remaining candidates are a stale bless
predating an unrelated signature change in `query.rs`, or a rustc I no longer have. **What is
measured and certain: 23 fixtures were red at `3163078f` under the mandated toolchain, and rung 10's
green certification did not cover them.**

Both causes are re-blessed here in one pass, under 1.97.1, and are listed separately above so the
diff is reviewable rather than a wall.

**RESOLVED 2026-08-11 (owner: "реши сам"). Both decided; shipped as
`tests/trybuild_corpus_compiler_witness.rs`.**

**1. NO `rust-toolchain.toml`. A COMPILER WITNESS instead — and the reason is that a
`rust-toolchain.toml` would not have caught this.** The shadowing binary is a **standalone**
`cargo.exe`/`rustc.exe` from chocolatey, not a rustup proxy; a standalone cargo ignores
`rust-toolchain.toml` outright, so the file would have looked like protection while providing none.
Worse, the only form of it that would fix the *other* half — pinning the host triple, `channel =
"stable-x86_64-pc-windows-gnu"` — breaks every non-Windows build of a workspace whose stated targets
are *"Windows / Linux (x86_64)"*.

What ships reads the compiler's own version string and compares it to a `BLESSED_RUSTC` const
updated **in the same commit as any re-bless**. It catches a toolchain update *and* the shadow, on
any host, and it fails with both versions named plus what to do about it. **Its two REDs were run:**
naming a compiler that is not running prints `blessed: 1.98.0 … running: 1.97.1 …`; raising the
fixture floor prints `claims to speak for at least 9999 … and found 90`.

**The precedent decides the shape.** This repository already freezes a compiler for a byte-exact
corpus: every committed `.spv` is gated against a **frozen `dxc` recipe in the shader's own header**,
so a compiler change cannot silently redefine the artifact. A `.stderr` corpus is that shape with a
different compiler and had no freeze. Now it does.

**MEASURED while writing it, and it corrected the entry above:** the corpus is **90 `.stderr`
files**, not the 24 this section first said. 24 was the number of files rung 11's *diff* touched. **A
count taken from a diff is a count of what changed, not of what exists**, and the two are equal only
by accident.

**2. `trybuild` STAYS.** A compile-fail fixture proves a property no runtime test can reach — that
the type system *rejects* a shape — and this rung leaned on exactly that for `G12` clause 3 and rung
10 for `G22b`. The price is real and is now **visible instead of silent**: when rustc changes, one
named gate fires and says so, rather than 23 fixtures mismatching under a green-looking sweep.

⚠️ **One coupling the witness does NOT remove, recorded because it is the surprising one.** A
`.stderr` corpus is coupled to the engine's **impl count**, not only to the compiler: past five
implementors rustc switches the *"other types implement trait …"* block from spans to an inline
list. Adding one trait impl anywhere can therefore re-render a pinned diagnostic in a crate that has
nothing to do with it. No gate can prevent that; the witness at least stops it being confused with
compiler drift, which is exactly the confusion it caused here.

---

## 2026-08-10 — RESOLVED (owner: "реши сам"): both rung-10 gate questions, decided and closed

The owner delegated these two rather than deciding them. Both are decided below, with the reason
each way was taken, and both entries in `05-LADDER-GATES.md` are updated to match.

### `G17` keeps the A/B ratio. No release-profile absolute-nanosecond gate.

**Decision: keep what ships.** The question was whether to build a release bench harness and pin a
per-box nanosecond floor so the corpus's literal thresholds (`static-armed ≤ 12 ns`,
`dyn-armed ≤ 14 ns`, …) could run.

An absolute-ns threshold is a claim about the **machine**, not about the code. The property the row
is actually protecting is *"the handle carries its arm bit, so the emission path never dereferences
`REGISTRY`"* — a structural property of the implementation, and one that holds or fails identically
on a 2 GHz laptop and a 5 GHz desktop. The A/B measures exactly that: both variants, interleaved,
one thread, one sitting, and a machine that is slow today is slow for both legs. A 2 ns budget over
12 would additionally have to be re-blessed on every box the repository is ever built on, and would
red-light on a busy CI runner for a reason having nothing to do with this code — which is the
failure mode a gate exists to *avoid*, not to demonstrate.

What is kept from the corpus's intent: the absolute figures **are printed**, with the build profile
named beside them, so a human can compare them to the row's numbers whenever they want to. What is
refused is *asserting* on them.

The one caveat that survives is stated at the file: the ratio was measured in `debug` (2.02×).
`cargo test --release` runs the same test and prints the release pair; nothing in the assertion
depends on which profile it ran under, so there is no separate release leg to build.

### `G22b` clause 2 is REWRITTEN against the real symbol, not deleted

**Decision: rewrite, narrowed to the property that can actually fail.**

Deleting it was the tempting option — the clause as written describes a failure the type system
makes unwriteable (`SyncCells<T, N>` takes its extent as a const generic, so no run-time
`ProfilerConfig` value can size one), so the claim is vacuously true. But deleting leaves the corpus
with **no clause naming `assert_zero_init_eligible` at all**, and rung 10 measured that this is
precisely the property that breaks: `ZoneDesc` carries a `&'static str`, cannot be `ZeroInit`, and
`DYN_DESCS` needed a `MaybeUninit` wrapper that reads like ceremony and is easy to delete.

So the clause now reads: *a `.bss` arena declared over a type whose all-zero bit pattern is not a
valid value must fail at compile time; delete `DYN_DESCS`'s `MaybeUninit` wrapper ⇒ `E0277` ⇒ red* —
which is the `trybuild` case already shipped at rung 10 and already blessed. The old sentence's
claim is kept as a **recorded impossibility** with the const-generic argument beside it, so a future
revision does not re-add a gate that cannot fail.

`G22b` clause 1 is untouched and remains BLOCKED on `rustup component add llvm-tools`.

---

## 2026-08-10 — RESOLVED: retire the whole S1.5 harness, phase driver included. Rung 7's mechanical gate is CLOSED.

Owner's answer to both scope questions was the same: retire. Shipped. `rg
'TimestampCollector|VbTimedPass|Sv0TimedPass' crates/` now returns **zero code matches** — the rung's
gate, open since the campaign began, is closed.

Two things worth knowing about what the deletion cost, neither of them a decision to make:

**The S1.5 A/B is gone as an experiment, not only as a printer.** `sv0_bench_lighting_flags` drove
`SHADOWS|AO` off on two frames in four; every frame now pushes what every non-bench frame always
pushed. The transcribed numbers stay in `sv0_deferred_term_bench.rs` with its device-free arithmetic
gates, and this repository's own rule applies to them: a result established on a retired instrument
bounds nothing about the current one. Any rung needing a CURRENT Deferred-marcher figure takes a new
measurement on the zone artifact.

**Rung 7 ends with no A/B gate on any GPU family.** `gbuffer_zone_port_gate.rs` went with its leg A,
as `vb_zone_ab_witness_gate.rs` did. That is correct — there is nothing left to compare — and it
means the stage tables under `zone_begin_stage` are pinned by `const` blocks rather than measured
against an independent copy. Named at the function, not left to be discovered.

---

## 2026-08-10 — Rung 7 step 6c attempted and REVERTED: the SV0 bench is not a printer, it DRIVES the A/B it reports.

You answered the previous item with "retire the harness", and that part is settled — the
`window_present_gbuffer.rs` timing leg goes. The deletion still did not land, and the obstacle is a
new one that only surfaced by attempting it. **The tree is back at the last green commit; nothing is
half-finished in it.**

Rung 7 step 2 deleted the VB *printed measurement channel* as pure output. The corpus carries that
same framing forward to the SV0 half, and it is wrong there. `runner.rs`'s S1.5 harness computes
`sv0_bench_lighting_flags` from an ABBA phase counter and threads it into the scene every frame — it
does not merely REPORT the interleaved A/B, it DRIVES it, by changing what the marcher shades.
Deleting the timing channel therefore deletes a **render-path input**, which is a different act from
deleting a printer.

So there is a second question, and it is yours for the same reason the first was:

* **Retire the whole S1.5 harness** — the phase driver with the printer. Its transcribed numbers stay
  in the plan; nothing in the tree reproduces them afterwards.
* **Keep the phase driver, delete only the timing** — the A/B still runs and still changes the
  frame, but nothing measures it. That is a scene input with no consumer, which this campaign has a
  name for: a value nothing can make move.

My recommendation is the first. The second leaves a mechanism whose only purpose was to be measured.

**This blocks the last two collectors and the rung's mechanical gate.** Everything upstream of it is
shipped and green.

---

## 2026-08-10 — Rung 7 step 6 is blocked on a SCOPE call: does `engine_grand_showcase_512_gpu_pass_cost` get ported, or retired?

The last two GPU collectors (`TimestampCollector`, `Sv0TimestampCollector`) cannot be deleted while
something reads their durations, and one thing does: the `#[ignore]`d offline printer
`engine_grand_showcase_512_gpu_pass_cost` in `crates/boyko_rhi_vulkan/tests/window_present_gbuffer.rs`.
Its sibling, `software_ray_baseline_cost.rs`, migrated in ten minutes — it turned out to be using the
collector as a plain array of query pools. This one genuinely reads per-pass timings.

**Why it is not simply a port.** `run_showcase_body_ddgi` builds ONE `GBufferScene` literal (~230
lines) and holds it across the whole timing loop. The zone leg needs `open_frame` (`&mut`) → a
shared borrow parked in `scene.gpu_zone` → present → `retire` (`&mut`), every frame. The shared
borrow's lifetime is in `GBufferScene<'a>`'s type, so setting the field to `None` between frames does
not release it and the `&mut`/`&` cannot alternate. `boyko_app`'s runner never hits this because it
rebuilds the scene every frame; this fixture would have to move a 230-line literal into the loop.

**Three ways out.**

1. **Rebuild the literal per frame.** Mechanical, contained, and makes a 230-line construction run
   200+ times where it now runs once. Nothing measures that construction, so the cost is unknown
   rather than negligible.
2. **Give `GpuZoneRecorder` interior mutability** so `open_frame`/`retire` take `&self`. `FrameSlot`
   already holds two atomics and an `UnsafeCell`, so this is the same change rung 5c made to
   `CommandWitness`, one level down. It is the most reusable answer and the one that widens the
   kernel's surface: anyone holding a `&GpuZoneRecorder` could then claim a ring slot.
3. **Retire the harness.** It is an `#[ignore]`d printer; the zone artifact carries the same four
   brackets; its numbers are already transcribed into the HW-RT plan. This is the option the corpus's
   own precedent points at — `vb_bench_totality_gate.rs` and `vb_zone_ab_witness_gate.rs` were both
   deleted rather than migrated once their subject moved.

**This is a SCOPE call, so it is yours.** My recommendation is **3**, and the reason is that 2 buys a
kernel capability for one caller that a per-frame scene rebuild already gives that caller for free —
but the harness is a published measurement channel for HW-RT R0, and deleting a channel is not mine
to decide. **Until it is answered, rung 7 step 6 stops** and the mechanical gate stays unsatisfiable.

---

## 2026-08-10 — Rung 7's mechanical gate would be satisfied by deleting the record of what it gated. Corrected in the corpus; disclosed here because it is a SPEC change.

The corpus gates rung 7 on `rg 'TimestampCollector|VbTimedPass|Sv0TimedPass' crates/` returning zero
matches. After the VB family's half of the deletion the tree has **zero code references and roughly
a dozen prose ones** — `gpu_zone.rs` explaining what its ten `ZONE_VB_*` constants are what remains
of, `command_witness.rs` reconstructing the rung-7c stage defect from the collector that carried it,
`vg_occ_split_timing.rs` naming the channel its table used to read.

`rg` cannot tell a surviving CONSUMER from a comment that records what was deleted and why.
Satisfying the gate literally means erasing exactly the measured history the campaign exists to
keep. **I scoped it to CODE** and wrote the reasoning at the gate — the same scoping the second
mechanical gate already has (`crates/*/src`). Recorded here rather than decided silently because it
narrows a specified gate, and a narrowed gate is the owner's to widen back.

## 2026-08-10 — Deleting the old VB collector deleted the only gate on the stage table. Stated, not repaired.

`zone_begin_stage` says which pipeline stage each VB zone opens at. It had a real gate while
`VbTimedPass::begin_stage` existed as an independently-written second copy: `G10`'s stage clause
compared the two stamp for stamp — 26 frames, 520 timestamps, all identical. That clause is what
caught rung 7c's silently-changed stages after five green commits.

Rung 7 step 5 deletes leg A, so the comparison has no second side. What replaces it is a `const`
block pinning each of the ten ids to the stage both tables agreed on. **It catches a row edited by
hand and cannot catch a bracket moved to a site where the other stage is the right one** — that
question is a measurement, and after this rung it belongs to rung 8. No action is requested; the
loss is named at the function it guards so nobody re-derives the table believing it is checked.

---

## 2026-08-09 — Rung 3d shipped two zones where the corpus specified a 90.8 KiB `RoundRecord` column. Reversible; the owner should know what it costs.

**A SCOPE call, disclosed rather than asked**, because the fork itself was a perf/architecture one
and those are mine to decide with numbers. What lands here is the one thing the numbers do not
settle: a specified public surface (`Profiler::rounds(back) -> &[RoundRecord]`) does not exist, and
that is the owner's to reverse if the lost quantity matters.

**What the corpus asked for.** `RoundRecord { frame, round, dispatched, begin, end }`, 24 B × 121
frames × `MAX_ROUNDS_PER_FRAME = 32` = **90.8 KiB** of the reservation, keeping *"dispatch shape
only: rounds per frame, wave width, round span"*.

**What shipped.** Two zone sites on the dispatcher — `__round` (Span) and `__round_width`
(Counter) — from which rounds per frame is `__round`'s `count`, round span is its
`total`/`min`/`max`, and wave width is `__round_width`'s. All three named quantities, per frame,
with distributions rather than a single row each.

**Why, in the order the reasons actually weigh.**

1. **The write path.** This is the decisive one and it is not an optimisation argument. The
   dispatcher does **not** hold `&mut EcsMaster` while a round is in flight — the `UnsafeEcsCell` it
   minted is shared with the workers. Writing a column from there needs either a second published
   pointer into the reservation, written by a thread the fold's `&mut` does not cover, or a
   per-schedule scratch buffer flushed after the run — profiling state owned by the scheduler. A
   lane push has neither problem and is the mechanism rung 3c already blessed for `SystemSpan`.
2. **No truncation.** `MAX_ROUNDS_PER_FRAME = 32` would have counted the 33rd round of a
   deep-dependency schedule as *dropped* rather than measured, with its own drop class. Two zones
   truncate at nothing.
3. **90.8 KiB and one drop class not spent.**

**What is lost, and it is a real thing.** The **correlation** between one round's width and that
same round's span — "was the widest round also the longest?" Per-frame aggregates cannot answer it.
Nothing in the profiling corpus asks that question today, which is why the call went this way.

**To reverse:** restore `RoundRecord` with a scratch-and-flush path in `ExecutorScratch`, accept
the 32-round truncation and its counter, and keep the two zones or drop them. Say the word.

*(Two smaller departures ride along and need no decision: `Interval.sys` is `Interval.zone`, because
`sys_of` is gone and zone → system resolves at report time; and `G8` has no SKIP clause, because
`ThreadPoolBuilder::num_threads(2)` never consults the machine, so fewer than two workers would be a
threadpool defect rather than an environment to excuse. Both are argued in
`docs/diagnostics/profiling/05-LADDER-GATES.md`.)*

---

## 2026-08-09 — The dev residency budget has 1.3 MiB of headroom, not the 9 MiB the corpus's table implies. `J1`'s `Z` contradiction, now with a number.

**Recorded, not asked** — it is `J1`'s to settle and was already logged at rung 3a. What is new is a
measurement, and the measurement changes how urgent it looks.

`profiling_residency` now prints its configuration. On this box, armed, analysis ON:
**total 14 667 776 B (reservation 14 614 528, statics 53 248)** against a 16 MiB dev budget.

The corpus's own dev rows are **6.67 MiB** (analysis off) and **7.05 MiB** (on) — roughly half. The
gap is *not* the new interval ring, which is 262 144 B of it. It is `D8`'s `Z = 1024` against the
shipped `ENGINE_ZONE_SLOTS = 4096`: the five columns come to 21 B × 4096 × 121 = 10 407 936 B where
the table budgets 21 B × 1024 × 121 = 2 601 984 B.

So the sizing table and the shipped constant have disagreed by a factor of four since rung 2, and
the consequence is that the dev budget is at **87 % utilisation** rather than the ~44 % the table
suggests. `J1` owns the fix — either `ENGINE_ZONE_SLOTS` comes down to 1024, or every sizing row
in `profiling/01-EMISSION-STORAGE.md` is recomputed at 4096 and the retail budget re-derived with
it. Nothing is blocked today; the headroom is just much thinner than it reads.

---

## 2026-08-08 — The whole `92xx` code block described the wrong eighteen conditions. Repaired; recorded because of HOW it hid.

**Recorded, not asked.** The repair direction was not a judgement call and it is already committed.
What belongs here is the mechanism, because it is a hole in this project's own gate design and it
will recur.

**What was wrong.** L2 reserved eighteen `92xx` rows for the profiler — correctly, and for a good
reason — and then wrote eighteen plausible summaries composed from the code *numbers*. Sixteen of
them name conditions the profiling corpus does not have. `W9207` is the sharpest case: the corpus
pins it as **invariant TSC absent** in five documents, and logging's own `W0101` was **struck in its
favour**, so the invented summary ("a GPU query pool returned fewer results than were issued") left
the engine's only invariant-TSC code naming something else while the condition it was struck for had
no code at all. `9213` is `E9213` in the corpus (six mentions, four files) and was seeded `W9213`.

**Why nothing caught it, and this is the part worth keeping.** The registry has seven checks and all
seven were green. A `Pending` row owes **no doc page** (check 2 is `Live`-only) and **no emitter**
(check 3a is `Live`-only) — both narrowings are correct on their own terms, and L2 argued for them
in writing: *"otherwise L2 would owe eighteen pages for codes with no emitters, which is doc-rot
manufactured by a gate."* That reasoning is still right. But the two narrowings together mean a
`Pending` row's summary is compared against **nothing**, by construction — and the registry's own
check-4 message already names this defect class: *"inventing a summary here is how three rows of
this registry came to disagree with the messages the engine prints."* The registry documented the
failure mode, then shipped it at six times the scale, in the one status where no check could look.

**The generalisation, which is not repaired and is the reason this is written down.** A `Pending`
row is a **promise with no gate on its content**. Today the only thing that will ever check a `92xx`
summary is the rung that flips it `Live`, i.e. between one and fourteen rungs from now. Rung 2 flips
seven of them and reads the other eleven; the remaining eleven are still un-compared, and if a later
rung flips one without re-reading the corpus, the invented sentence ships.

**Two ways to close it, neither taken here** (both are bigger than rung 2 and one is a VALUES call):

* **(a) A check that pins every row's summary against the corpus.** Mechanically: for each `92xx`
  row, require its condition text to appear in `docs/diagnostics/profiling/05-LADDER-GATES.md`'s
  §Integration list. Cheap, and it would have caught all sixteen. Cost: it couples the registry's
  wording to a document's wording, which is a second statement of one fact — the exact shape this
  corpus deletes elsewhere.
* **(b) Do not seed summaries at all** — a `Pending` row carries the rung and an empty summary, and
  the summary is written when the emitter is. Structurally correct, and it is what
  `FORWARD_DECLARED` already does for the seventeen logging codes. Cost: `explain()` returns nothing
  useful for a `Pending` code, which is a real loss for anyone reading a corpus document today.

**Blocks nothing.** Rung 2 landed with the summaries repaired and seven rows flipped `Live` with
their pages. The other eleven are correct as of this commit and un-gated after it.

---

## 2026-08-08 — L5 shipped, and it weakened a specified latency bound. No decision needed unless you want it back.

**Recorded, not asked.** This is an architecture fork I decided with the tree in front of me; it is
here because it makes a *specified* number worse, and a number that quietly got worse is the thing
this file exists to prevent.

**The situation.** The logging corpus says `log_drain_system` runs "in `Last`", and states the
in-frame latency bound as **one frame** under `Scheduled` / "sink park + one frame" under `Thread`.
This engine has no `Last`. `CoreSchedule` is a **closed set of two** (`Main`, `Fixed`) and its own
doc gives the intended answer — *"finer-grained structure WITHIN a schedule is what Phase-15 sets
are for."*

**What shipped.** The drain runs in `Main`, `in_set(LogSet)`, and `LogPlugin::build` interns the set
so a host's `.before(LogSet)` resolves regardless of plugin add-order.

**What it costs.** With no ordering edge the scheduler may place the drain anywhere in the frame, so
a record emitted *after* it appears in the next frame's ring. **Each specified bound gains one
frame** for a host that does not add the edge. The drain has no data conflict with anything, so
nothing forces it late on its own.

**Three ways to get the frame back**, none taken, because all three are bigger than L5:

* **(a) Do nothing** — document the edge and let each host add `.before(LogSet)`. What shipped.
  Costs one line per host; costs a frame if a host forgets, silently.
* **(b) Add a third `CoreSchedule` variant.** Honest, matches the corpus verbatim, and touches the
  frame driver, the routing methods and every `add_systems_in` call site. An engine change to serve
  one subsystem.
* **(c) Give the engine a standing `EngineSet::Last`** that the app plugins order everything before.
  Cheaper than (b) and useful beyond logging, but it is a scheduling convention the engine does not
  have yet, so introducing it from the logging seam is the tail wagging the dog.

**Blocks nothing.** L16's `G15` is where the bound is actually measured, and it must be measured
against whichever of these is true then.

---

## RESOLVED 2026-08-08 — the owner chose (b): raise the budget. Q1 stands, no code changes.

**Decision:** accept the footprint and raise the gate's shipping budget from **1024 KiB to
1280 KiB**. `LANE_COUNT` stays 80 in every profile; `REGION_CAPACITY` stays 128; D15 keeps
committing the sample slab at first `arm`. **No source change of any kind.**

**Why it is the right call, in the owner's own framing.** The alarming number measured a
*reservation*, not what a machine holds. The row is now printed in three columns instead of one
that conflated them:

| Column | Shipping profiler | Meaning |
|---|---|---|
| declared / reserved | **1 208.2 KiB** | address space; free in any practical sense on 64-bit |
| committed at `arm` | **≈ 1 142 KiB** | the reservation, taken when diagnostics are turned **on** |
| resident, flag off | **≈ 0** | nothing armed, no lane claimed, no `.bss` page touched |

The owner asked whether this costs runtime or RAM. It costs neither in the shipped default: with
the flags off a site pays **one `.bss` byte load and one predicted branch**, and above the compile
ceiling it pays nothing at all — the site and its argument expressions are deleted. That per-site
floor is the only thing a runtime flag cannot remove, and no budget choice touches it.

**Consequences applied:** `G23a` and `G23b` are **unblocked** — they assert against 1280 KiB and
have a reachable green state again. Options (d) `REGION_CAPACITY = 64` and (a) per-lane lazy commit
are **retained below as levers**, not as work: pull one only if a measurement later says the
*committed* figure is too high.

*(Original entry follows, unedited — the record of why the call was made outlives the call.)*

---

## 2026-08-08 — ⚠️ Q1 raised the shipping diagnostics footprint by 1.07 MiB, and the profiler's "≤ 1 MiB retail" headline is now FALSE

**This supersedes the ≈ 2.08 MiB figure in the round-3 entry below.** The correct joint figure is
**≈ 3.15 MiB**.

**What happened.** At rung D0 I resolved architect blocker **Q1** by deleting `LANE_COUNT`'s build
profile axis — it was 32 in the shipping profiles while the quantity it indexes,
`boyko_threadpool::MAX_WORKERS = 64`, is unconditional, so 32 was unsound and below the topology's
own floor of 66. The resolution (80 in every profile, `455c074`) was correct and I stand behind it.
**What I did not do at the time was propagate its cost**, and four cells across the two plans were
sized by that constant:

| Cell | Was (32 lanes) | Is (80 lanes) | Kind |
|---|---|---|---|
| profiling `LANES` (`.bss`) | 8 KiB | 20 KiB | reserved |
| profiling **sample slab** | 192 KiB | **480 KiB** | **committed at first `arm`** |
| logging `LOG_LANES` (`.bss`) | 512 KiB | 1.25 MiB | reserved |
| logging `SAMPLE_CTR` (`.bss`) | 16 KiB | 40 KiB | reserved |

Profiler half **908.2 → 1 208.2 KiB**; logger half **1 220.26 → 2 012.26 KiB**; joint
**2.08 → 3.15 MiB**. The `dev` figures did not move at all, because `dev` was already at 80 —
**which is exactly why nothing caught this: every check that looked at one row looked at the row
that was still right.**

**Two things follow that are not "the number got bigger".**

1. **The profiler is now over its OWN budget**, not just the joint one: 1.18 MiB against a stated
   ≤ 1 MiB. Gates **G23a and G23b assert that bound**, so both now fail at the baseline — they have
   no reachable green state until this is answered.
2. **Only 288 KiB of the +1.07 MiB is committed memory.** The rest is `.bss` reserved extent whose
   resident cost is per *touched* lane, which is the property that made 80-everywhere affordable in
   the first place. The committed part is entirely the profiler's sample slab, which **D15** commits
   for all `LANE_COUNT` lanes at first arm.

### The call

> **RECOMMENDATION, final: (b) — raise the budget and restate the row honestly.** The owner asked
> the right question: *is 3.15 MiB actually a lot?* It is not, and more importantly **the figure
> measures the wrong thing.** It is a **reservation**. What a machine actually holds is:
>
> | Configuration | Resident |
> |---|---|
> | shipped title, diagnostics flag OFF (the default) | **~0** — every table is demand-zero `.bss` that nothing touches, nothing is armed, nothing is committed |
> | shipped title, diagnostics ON | the profiler's sample slab (480 KiB, committed at `arm`) plus the logger's *touched* lanes and staging — order of **1 MiB**, not 3.15 |
>
> Address space on 64-bit is free in any practical sense, and 1 MiB resident against a single
> 2048² RGBA8 texture at 16 MiB is not a trade worth buying with code. So the cheapest fix of all
> is the one that changes no code and no constant: **raise the gate's shipping budget** (1024 →
> 1280 KiB covers 1208.2 with headroom) and print the row in three columns — reserved,
> committed-when-armed, resident-when-off — instead of one number that conflates them.
>
> That also unblocks G23a/G23b immediately, which (d) and (a) do only after a code change.
> **(d) and (a) below are kept as the levers to pull if a measurement later says the committed
> figure is too high**, not as things to do now.

- **(d) — kept as the cheap lever, no longer the recommendation.** Set `REGION_CAPACITY = 64` in the shipping
  profiles instead of 128. The slab is `LANE_COUNT × 2 regions × REGION_CAPACITY × 24 B`, so
  `80 × 2 × 64 × 24` = **240 KiB** instead of 480, and the row lands at
  `66 + 240 + 636 + 6.8 + 11.4 + 8` = **968.2 KiB — under the 1024 KiB budget**, with Q1 intact.

  **Cost:** a region holds 64 samples instead of 128 before the fold must drain it, so a shipping
  build drops samples earlier under a burst. In shipping the tier is `Always` only, so the sample
  stream there is already an order of magnitude thinner than in `dev`.

  **What it does not cost:** not one line of code, not one branch on the hot path, not one gate
  re-specified. `REGION_CAPACITY` is *already* a per-profile constant — unlike `LANE_COUNT`, whose
  axis Q1 deleted as unsound — so this is the knob doing the job it exists for.

  *I proposed (a) first because I was looking at where the bytes are rather than at what is
  cheapest to remove. The order is the other way round: the constant that exists for this, then
  code.*

- **(a) — now the fallback, if measurement later shows 64 samples per region is too shallow.**
  Commit sample regions **per lane on first use** instead of all 80 at
  arm.

  **Performance is not the problem with (a); the gate is.** The hot path already loads
  `buf: AtomicPtr<Sample>` on every sample, so a null test is one `test`+`jz`, predicted
  not-taken after a lane's first sample — call it zero. The commit itself is one syscall per lane
  on a `#[cold]` path, ten of them over a process. Two real costs, though: the syscall lands
  **inside a frame** (a worker's first zone, during the first frames, where frame times are
  already noisy — mitigable by committing at `arm()` for lanes that already exist, since the pool
  is built before `arm`), and it adds unsafe surface to the profiler's hottest path. The one that
  matters most: **G23a/G23b stop being able to assert a single armed total.** The figure becomes
  warm-up-dependent, and a crisp gate becomes a "after N frames" gate. A shipping title on an 8-core box claims roughly `workers + dispatcher + host ≈ 10` lanes, so
  ≈ 60 KiB of slab instead of 480 — the profiler is back under 1 MiB **with Q1 intact and no
  constant changed**. It edits D15 ("committed once at first arm, never freed"), which is a
  shipped-behaviour decision, which is why it is yours and not mine.
- **(b)** Accept 3.15 MiB reserved / ~1.2 MiB committed and restate the headline.
- **(c)** Cut a table instead: logging's `LOG_LANES` (1.25 MiB), `SINK_OUT` (256 KiB), or the
  profiler's dynamic-zone arenas (96 KiB per the profiling plan, 40 KiB per `SEAM.md` — that
  divergence is still open and is the profiling plan's to close).

**Blocks:** profiling rungs 2 and 10 (G23a/G23b). Does **not** block logging L0 or anything on the
substrate ladder.

**The lesson, recorded because it is the fourth time this shape has appeared in this campaign:** a
total that is a perfect sum of its printed operands proves nothing about whether the *operands* are
current. 2.08 MiB was correct arithmetic over two halves the substrate had already invalidated. The
check that would have caught it is not "does the total add up" but "has anything this total depends
on been decided since it was written".

---

## 2026-08-06 — ⚠️ MEASURED: synchronization validation is not live, so the `-ValidationOn` leg proves nothing about barriers

**A genuine missing barrier changed no pixel and emitted no message.** Executed while resolving
piece 2's first step, which existed precisely to find this out.

The probe: delete the ONLY declared read of a resource with exactly one reader — the HZB pyramid's
mip `d-1` read — while the dispatch that reads it stays. Pass 0 writes mip 5, pass 1 reads mip 5, no
derived dependency.

| | messages | `SYNC-HAZARD-*` | golden |
|---|---|---|---|
| baseline (×2, same build) | 19 | — | byte-identical |
| **real missing barrier** | **19** | **none** | **byte-identical** |

The feature bit IS requested in `boyko_rhi_vulkan/src/device.rs`, but the instance chain degrades
**silently** when `VK_EXT_validation_features` is absent, and the whole 19-message baseline is
`vkCreate*`-time — nothing in it was ever produced by a recorded frame.

**Why this is here rather than merely recorded.** It is not a piece-2 fact. It says that the
engine's validation leg — the instrument this campaign has been leaning on since the P1-2
`-ValidationOn` repair — covers object, descriptor and format legality and **nothing about
synchronization**. Every "validation clean" claim in the campaign's commit messages is true and
narrower than it reads.

**Options.** (a) Leave it, and gate barrier correctness structurally (pin the derived barrier stream
by FIELDS, which is what piece 2's G4 now does). (b) Find out whether `VK_EXT_validation_features` is
genuinely absent on this device or merely not reaching the layer, and fix it if it is the latter —
this is a ~1-hour investigation and would restore a general-purpose instrument. (c) Both.

**My recommendation is (c), with (a) first**, because (a) is already specified and blocks nothing,
while (b) is worth doing before piece 3 — that piece adds the first pyramid READER, and a
read-after-write across two passes is exactly the hazard class the layer would catch and the golden
cannot.

⚠️ **A methodological note worth as much as the finding.** The FIRST probe was inconclusive by
construction: it deleted one of SIX declared readers of the same image, so siblings still carried
both the transition and the dependency and nothing was tested. Its negative result would have been
recorded as "the extension is absent on this device" — a true statement reached by an invalid route.
When probing for a missing dependency, count the OTHER declared accesses to that resource first.

---

## 2026-08-05 — CI's release leg is red, and two of the classes are STRICTNESS calls

Found while preparing the P1-5a baseline, which needs a leg that passes. Running CI's own command —
`cargo test --workspace --all-targets --release --exclude boyko_demo --exclude bench-bevy-vs-boyko`
([ci.yml:62](../.github/workflows/ci.yml), `:103`) — **fails on six targets**. Two more appear on a
second run, which is itself the diagnosis: those are flaky, not release-specific.

Six were mechanical and are **fixed**: five `#[should_panic]` tests over `debug_assert!` guards
missing `#[cfg(debug_assertions)]` (boyko_math, boyko_ecs, boyko_render ×3, plus two in
boyko_rhi_vulkan), and one missing `VB_PINS` entry that was mine — `vb_mesh_hzb` from VG R3 P1-2.

Two classes are left, and both are decisions about how strict a gate should be rather than
architecture forks, so they are here rather than taken.

### 1. `boyko_shaderdsl --test eval_byte_identity` — three failures, on the NaN SIGN BIT alone

`NaN (0xffc00000)` vs `NaN (0x7fc00000)`. Both quiet NaNs; the values are identical (a NaN is not
even equal to itself); only the sign differs.

This is the same family as what gate G3 measured on the depth pyramid the same day: **the sign of a
zero and the sign of a NaN are exactly the two bits no `<` in a program can observe**, which is why
hardware and optimisers are free to move them — G3 caught a driver fusing a compare-and-select into
a hardware min whose ±0 tie-break differs. Expecting either bit to be stable between `-O0` and `-O3`
is not well founded.

**Options.** (a) Compare NaN as "both are NaN" rather than by bits. (b) Canonicalise the sign before
comparing. (c) Leave it, and accept that the eDSL's release leg is not a gate.

**My recommendation is (a).** The contract the eDSL exists to enforce is about VALUES, and the sign
of a NaN is not a value. It costs nothing on the finite domain, which is the whole domain that
matters, and it stops a real gate from being permanently red — which is worse than a slightly
narrower one, because a red gate nobody can fix is a gate nobody reads.

### 2. Two global-state tests that are flaky under parallel execution

`boyko-scene bundles_s6::interner_is_off_the_per_frame_path` reads the process-global
`identity::interner_len()`. `boyko-ui zero_alloc::unchanged_frame_layout_pair_allocates_zero_over_baseline`
reads a global allocation counter — and reported a delta of **minus one**, an improvement its
`assert_eq!` cannot express while its own message says "no more than".

Both pass alone, pass under `--test-threads=1`, and pass in debug. They fail only in release with
default parallelism.

**Options.** (a) A serial guard in each test file. (b) `--test-threads=1` for those binaries in CI.
(c) Make the UI assertion `<=`, matching its own message, and serialise only the scene one.

**My recommendation is (a) plus the `<=` repair**, because the harness flag would slow every test in
those crates to fix two, and because an equality assertion that fails on an improvement will fail
again the next time somebody improves it.

---

## 2026-08-05 — SCOPE: the pyramid needs a core framegraph change (per-subresource sync state)

**Decided and under way, not blocking — recorded because it grows piece 1 beyond what its plan
scoped.** Step P1-5 (declare the HZB build passes) cannot be written against the framegraph as it
stands, and the framegraph says so itself.

**The wall.** `framegraph/graph.rs:360-445` carries `INVARIANT HZB-SUBRESOURCE-UNIFORM`: every
access to one `ResId` must declare the same `(base_mip, mip_count, base_layer, layer_count)`,
because `FrameGraph::state` is a `Vec<ResSync>` indexed by `ResId` alone and `transition` never
receives the span. The HZB build needs, on ONE image in ONE pass, a read of mip `6p-1` and a write
of mips `[6p, 6p+n)`.

That comment was written in advance, names this exact pass, and prescribes the answer:

> "PER-SUBRESOURCE TRACKING IS THE CORRECT LONG-TERM ANSWER, and this assert is its TRIGGER … the
> HZB build writes mip k while reading mip k-1. When that pass is authored, it trips this assert.
> That is the INTENDED way to discover the work … The response is to build per-subresource
> tracking, never to relax the condition until it goes quiet."

**It is not merely a debug assert.** In release the assert is compiled out and the derivation is
genuinely wrong, traced at 512×512: pass 0 first-touches mips [0,6) so only those leave `UNDEFINED`;
pass 1 then writes mips [6,10) with the state claiming GENERAL for the whole ResId, so the derived
barrier has `old_layout == new_layout` and mips 6..9 are **never transitioned** while the dispatch
writes them through storage descriptors declared `GENERAL`. Every extent with
`prev_pow2(max(W,H)) >= 64` reaches it.

**The workaround I rejected.** Three ResIds aliasing one `VkImage` over disjoint mip spans is
uniform by construction and needs no framegraph change. I turned it down for two reasons. It is
literally what the invariant's own text forbids ("not by making the declarations agree by hand"),
and it is a dead end one piece later: piece 3's cull selects a pyramid LEVEL per instance, and a
per-pass ResId cannot be named by a dynamic level. Taking it would be the interim design deferred to
later that this project has ruled out.

**What it costs.** A new step P1-5a ahead of P1-5, touching the core state machine every render path
compiles through. The byte-identity argument is strong — `SubRange::color_mips` is called by nothing
today and every existing `image_access` site passes `base_mip: 0, mip_count: 1`, so a per-mip
machine should fold to today's behaviour barrier-for-barrier — but "should" is why it gets its own
gate rather than a golden pin, which cannot see a redundant or a missing barrier.

**Nothing is blocked on an answer.** Architecture forks are mine to decide; this is here because the
SCOPE grew, and the owner may prefer piece 1 to stop at "allocated and compiled" and hand the
framegraph work to its own campaign. Say so and I will split it.

---

## 2026-08-04 — ⚠️ `golden.ps1 -ValidationOn` never enabled the validation layer on ANY `boyko-app` pin

**Found while gating VG R3 P1-2, and it is the vacuum-green shape again.** The switch is the
engine's validation-audit instrument; the campaign records a "Validation-ON audit — COMPLETE"
milestone that ran through it. On the 22 of 25 pins that boot through `boyko_app`, it could not
fail.

**Mechanism.** The backend gates the layer on a conjunction —
`config.enable_validation && BOYKO_DISABLE_VALIDATION unset` (`boyko_rhi_vulkan/src/device.rs:2350`)
— and `boyko_app`'s runner hardcoded `enable_validation: false`. `-ValidationOn` only ever
*stripped the env var*, i.e. satisfied the second conjunct while the first stayed false. The layer
was never requested, no messenger existed, and the scan for `[vk-validation]` lines therefore
reported **"clean (0 messages)"** unconditionally.

The runner's own doc said so, in a passage read as a design note rather than as a gate defect:
"The shipped runner does NOT request the validation layer… a debug validation knob arrives with a
later rung."

**Measured, not inferred.** I built a `512×512` image with `mip_levels: 12` (the legal max is 10).
`vkCreateImage` returned SUCCESS and the audit reported clean. With the fix, the same corruption
draws the exact message: `vkCreateImage(): pCreateInfo->mipLevels (12) must be less than or equal
to 10`.

**Fixed here**, because P1-2's load-bearing gate depends on it: `BOYKO_ENABLE_VALIDATION` opts the
runner in (absent ⇒ boot byte-identical to before), and `-ValidationOn` now sets it alongside the
strip.

**⚠️ WHAT IT REVEALED, AND THE OPEN QUESTION.** With the layer actually live, the `vb_mesh` pin
emits **19 validation messages** — a baseline nobody has seen:

| count | message |
|---|---|
| 9 | `vkCreateComputePipelines()`: compute shader uses descriptor `[Set 1, …]` |
| 6 | `vkCreateGraphicsPipelines()`: vertex attribute at location 1/2 not consumed by vertex shader |
| 1 | **`vkDestroyDevice(): VkDevice has 13 leaked objects that have not been destroyed`** |
| 1 | `vkCreateShaderModule()`: SPIR-V capability `Geometry` declared without the feature |
| 1 | `vkCreateShaderModule()`: SPIR-V capability `DemoteToHelperInvocation` declared without the feature |
| 1 | duplicate-limit warning |

The pyramid is not implicated: armed and unarmed logs are **byte-identical** after handle
normalization, so P1-2 contributes zero. But the two shader-capability messages and the 13 leaked
objects are real, they are on the flagship VB pin, and one of them is a resource leak.

**The question is SCOPE, not method.** Options: (a) I audit and fix the 19 now, before continuing
the pyramid — it is a leak and two feature-declaration bugs on the main path; (b) I finish piece 1
and take the validation baseline as its own campaign afterwards; (c) I fix only the leak now and
defer the rest. I lean (b): the 19 predate this work by a long way, the pyramid is proven clean
against them, and interleaving an unbounded audit into a decomposition that was created
*specifically* to keep scope local would undo the decomposition. But it is your call — this is
scope, and the leak is the kind of thing that gets worse while it waits.

---

## RESOLVED 2026-08-03 — the HZB feature design does not converge in one piece: decomposed

**Situation, measured over three review rounds rather than felt.**

| round | prior items closed | new blockers | new majors |
|---|---|---|---|
| 1 | — | 8 | — |
| 2 | 10 YES / 6 PARTIAL / 0 NO | 3 | ~12 |
| 3 | 31 YES / 12 PARTIAL / 2 NO | 6 | 11 |

Each round genuinely resolves most of what the last one raised, and each raises about as much
again. After round 3 **every substantive step carries a blocker** (S4, S6a, S6b, S7, S9); the four
clean steps are gates and records that depend on the blocked ones. So unlike the foundation case
there is no independent subset to land.

The new blockers have also changed CHARACTER, which is the useful signal. They are no longer "the
algorithm is wrong" — they are collisions with shipped invariants: a fourth route by which the
design disarms rung R2d-6 (doubling the survivor list breaks the very const-assert added in R2d-4
to prevent an out-of-bounds device read); an `+INFINITY` fixture vertex reaching a second, unfenced
host consumer on the shipped VB path; a capability that is a per-frame ECS fact gating objects
minted at boot with no seam named between them.

**What I read from that.** The feature is simply larger than one design pass can hold. The
foundation converged in a single round each because S1/S2/S3 were small, independent and
self-contained — not because the process was better there.

**Proposal.** Decompose the feature the way the foundation already was, and give each piece its own
design + review round:

1. **The pyramid alone** — allocate, build, gate against the S3 host oracle. No cull integration of
   any kind. It is self-contained, its oracle already exists, and its own blockers are local.
2. **The capability and the raster split alone**, inert — the second scope drawing nothing, proven
   byte-identical on the pins.
3. **The cull integration**, once 1 and 2 are shipped and the collisions above are concrete rather
   than predicted.
4. **The arming**, with the drawn-set gate.

**The cost, stated.** Four design rounds instead of one, and the feature lands later. **The
alternative cost**, also stated: a fourth whole-feature round that on this evidence resolves ~30
items and raises ~6 more.

**OWNER'S DECISION: decompose.** The four pieces above each get their own design + review round
and land independently, starting with the pyramid alone. Recorded so a later reader sees the four
rounds were a deliberate structure, not a design that kept failing.

**Blocks.** Nothing.

---

## RESOLVED 2026-08-02 — the depth-complexity fixture is Khronos Sponza (delegated to me)

**Situation.** Decision (b) above commits to a separate fixture with real occlusion. What it should
contain was not decided, and it is an asset question I raised rather than settled on my own — the owner then delegated it back to me.

The VG corpus is seven fetched Khronos glTF sample assets arranged on a 5x3x3 grid — chosen for
triangle density, and it has almost no occlusion by construction. A depth-complexity fixture wants
the opposite: large occluders with substantial geometry behind them. The classic choices are Sponza
(an interior with a colonnade that occludes heavily) or Bistro; neither is in the manifest today.

**OWNER'S DECISION: delegated to me.** Chosen: **Sponza, from the same Khronos glTF-Sample-Assets
family the density corpus already draws on**, fetched into `assets/vg_occlusion/` under its own
manifest, gitignored and content-pinned exactly as `assets/vg_corpus/` is.

**Why Sponza and not Bistro or Intel Sponza.**

- *Same source family as the existing corpus*, so the fetch script's shape, the licence posture and
  the gitignore precedent all transfer. No new infrastructure and no new licensing question — the
  three things that would otherwise make this a multi-session detour.
- *Same loader path*: glTF/`.glb` through `GlbMeshLoader`, already exercised by every corpus asset.
- *It has the right OCCLUSION STRUCTURE*, which is the whole point: a colonnade plus an upper
  gallery, so a camera at floor level down the nave has its far half hidden behind columns. That is
  exactly what the density corpus lacks by construction.
- *Size*. This session already hit zero free space at a 73 GB `target/`. Bistro is ~2.4 GB, and its
  glTF conversions vary in provenance — which matters more here than usual, because this repo pins
  by content hash and a pin on an artifact nobody can re-derive is not a pin.
- *Comparability*: Sponza is the published occlusion/GI benchmark, so a number measured on it means
  something to a reader outside this repository.

**The risk I am taking, stated rather than discovered later.** ONE scene is the same
vacuous-selection exposure the corpus notes warn about — a single framing can be chosen to flatter.
Mitigation is the corpus's own: several committed camera paths spanning degrees of occlusion (down
the nave = heavy; from the gallery = moderate; outside looking in = little), with the WEAKEST
binding, exactly as `orbit_mid` binds the density corpus. A win claimed off the heavy framing alone
would be the defect, not the fixture.

**Still to do when it is built.** The `source_url` / `archive_sha256` / per-file `glb_sha256` pins
are filled from the first verified fetch, the way `CORPUS.toml`'s were — not written from a guess.

**Blocks.** Any occlusion perf claim. Blocks no implementation work, and is not on the HZB critical
path.

---

## RESOLVED 2026-08-02 — Occlusion perf claim: option (b), a separate depth-complexity fixture

**Situation.** The VG corpus is a triangle-density instrument, deliberately recomposed at rung R0b′
to measure density rather than occlusion. Measured ceiling on it: **1 of 44 drawn instances at
`orbit_mid`** (the binding framing) and 11 of 31 at `approach_close`. A min-reduced HZB can only
reject instances that win zero pixels, so those are hard upper bounds — and they bound more than
occlusion, since an instance also wins zero pixels when it is sub-pixel.

So the HZB and two-pass occlusion work now in flight can be built correctly and gated for
correctness, but **no occlusion speed-up can be demonstrated on any content in this tree**.

**Options.** (a) Ship it correctness-gated with no perf claim, as rung R2d shipped structural —
honest, and leaves the claim unmade until content exists. (b) Add a scene with real depth complexity
(an interior, a street) as a *separate* perf fixture, kept out of the density corpus so the two
instruments cannot contaminate each other. (c) Accept the claim will be made by whatever project is
built on the engine, not by this repository.

**OWNER'S DECISION: (b).** Build a scene with real depth complexity — an interior or a street — as
a SEPARATE perf fixture, deliberately kept out of the density corpus so the two instruments cannot
contaminate each other. Until it exists, the HZB rung ships correctness-gated with no speed claim.

**Follow-on, and it needs an asset decision** — recorded as its own item below rather than assumed.

---

## RESOLVED 2026-08-02 — K2: option (c), stays deferred

**Situation.** The virtual-geometry campaign's kill criterion K2 requires a Nanite reference table.
It has never been produced (UE is not installed; I cannot install it — the flow requires accepting an
EULA and creating an account, which I must not do). K2's own text says an unproducible baseline
*forces a scope restatement*: an absolute target instead of a relative one.

**Options.** (a) Owner installs UE and produces the table. (b) Restate the goal against an absolute
target (frame time at a stated triangle count and error bound) and record K2 as taken by its own
escape hatch. (c) Leave deferred and keep the goal formally unfalsifiable.

**OWNER'S DECISION: (c).** Stays deferred; the goal remains formally unfalsifiable, knowingly. No
rung is blocked by it. Recorded rather than quietly dropped, so a future reader does not mistake the
campaign's silence on K2 for K2 having been satisfied.

---

## RESOLVED 2026-08-02 — Cross-frame occlusion soundness

**The worry was mine and it was misframed.** A previous-frame pyramid is indeed not conservative —
but only for a ONE-pass cull. In two-pass it is never the last word: soundness lives entirely in the
late pass, which tests against a pyramid built from THIS frame's depth. The early pass is an
*unverified heuristic* whose only job is to fill the depth buffer with a good occluder set; its
mistakes cost late-pass work and never cost geometry. The theorem quantifies over every possible
early-pass output, so nothing about the early pass has to be proven at all.

Confirmed against practice rather than assumed: UE5 Nanite, Assassin's Creed Unity (SIGGRAPH 2015),
Granite, Bevy 0.16 and Unity 6's GPU Resident Drawer all have this same structure.

Full statement and proof: [VG-R3-HZB-PLAN.md](VG-R3-HZB-PLAN.md) §1. **No owner decision needed.**

---

## RESOLVED 2026-08-02 — HZB: option (a), then REVISED TO (b) — foundation first

**Situation.** With soundness settled, the implementation design was reviewed and returned REJECTED
by both reviewers. The blockers are real, not stylistic — among them: the design revives
frustum-culled instances and thereby deletes rung R2d-6's arming; unknown mesh bounds produce a
PERMANENT false reject for any streaming-in mesh, surviving both passes; the one gate that can see a
false reject cannot be built as specified, because `vb_depth` carries no `TRANSFER_SRC` and the
readback path it depends on is listed UNVERIFIED while being load-bearing; and the pyramid build,
being compute, must split the VB raster's single dynamic-rendering scope in two, which the plan does
not address — a naive second scope would `LOAD_OP_CLEAR` away the early pass.

Full list: [VG-R3-HZB-PLAN.md](VG-R3-HZB-PLAN.md) §5.

**Options.** (a) One more design revision round against the 8 blockers, then implement — the same
loop that took rung R2d from 8 blockers to shipped. (b) Implement the uncontroversial foundation
first (S1 the RHI `TextureView`, S2 the framegraph guard, S3 the host oracle) while the cull design
is revised — these three are independently useful and none depends on the disputed parts.
(c) Park the rung.

**OWNER'S DECISION: (a).** One full revision round against all 8 blockers, then implement in step
order. My own recommendation had been (b) — land the uncontroversial foundation first — and it was
not taken; (a) is the same loop that carried rung R2d from 8 blockers to shipped, and it keeps the
step order intact rather than interleaving foundation work with a design still in motion.

**REVISED TO (b) the same day, by the owner, after the revision round returned.** The round closed
every prior blocker (10 YES / 6 PARTIAL / **0 NO**) and produced 3 NEW blockers plus a dozen majors
— and every one of them lands in the FEATURE: candidate routing, a capability predicate missing its
`mesh_leg` conjunct, a boot clear needing a `TRANSFER_DST` the image is not created with, an
un-ringed per-frame UBO, an unobservable `prev_view_proj`, an unexecutable anti-vacuity clause.

**Not one lands against the foundation** — the RHI `TextureView`, the framegraph subresource guard,
or the host oracle. Three design rounds, zero blockers there. That is evidence rather than
preference, and it inverts my original reason for recommending (b): it was a hunch then, it is a
measurement now.

Some blockers exist BECAUSE the foundation does not: one says outright that a step's acceptance
cannot be executed at that step because the instrument does not exist yet. Building the foundation
first removes a class of objections rather than postponing it.

**So: implement S1 (RHI `TextureView`), S2 (framegraph subresource guard) and S3 (host oracle) now.**
Each is independently correct, needed regardless of whether occlusion culling is ever armed, and
none depends on a disputed part. The feature's design continues to settle against its remaining
blockers, on a foundation that by then exists.

**Blocks.** The occlusion feature only. The foundation proceeds.

---

## 2026-08-07 — TWO THINGS PIECE 3 CANNOT DECIDE FOR ITSELF

VG R3 piece 3 is COMPLETE and pushed (`b6337dd`..`6a9a7f9`). Two items are blocked on the owner,
and neither is a defect.

### 1. Four new pins are UNBLESSED, and blessing is not mine to give — **RESOLVED: blessed @e160434**

**The owner reviewed the BMPs and signed off; all four legs now record
`85b7d378…4d2913d9` and re-verify green.** The review raised one real question — the corner
spheres look stretched — which was investigated before blessing, not waved through: the
silhouettes are ellipses with RADIAL major axes at `1/cos θ ≈ 1.18` (FOV_Y = 52°), and a
pixel-exact Bevy 0.14 replica of the two corner spheres reproduced the same ellipses to 0.2 px.
Rectilinear perspective, not a defect. The original record follows.

`goldens/PINS.toml` gained `vb_occ_mixed_off`, `vb_occ_mixed_keep`, `vb_occ_mixed` and
`vb_occ_mixed_late`, every `sha256_*` seeded with the literal `PENDING`. That is the path the file's
own header prescribes for adding a leg by hand; `golden.ps1` reports "NO PIN recorded" and exits 2
on all four rather than passing. **Verified, all four.**

All four render the SAME image:

    actual = 85b7d3788130a8bb65f0b5b92ba86c71499bd7a4babe7d6900a711944d2913d9

That identity across four regimes — disarmed, FORCE_KEEP, armed (defers 4), FORCE_LATE (defers 6,
re-admits 2) — is the piece's central claim: the cull rejects geometry and the picture does not move.

**What is needed:** a visual sign-off on the freshly-dumped BMPs. Then bless `vb_occ_mixed_off`
first (both legs) and verify the other three reproduce the same literal;
`the_pins_declared_byte_identical_actually_agree` keeps them from drifting afterwards. Until then
those four gates claim nothing — `PENDING == PENDING` is vacuous, and the guard's own doc now says so.

### 2. Piece 4 has no plan, and its scope is a VALUES call — **RESOLVED: planned @799db99, shipped P4-1…P4-7**

**Both halves are answered.** `docs/VG-R3-P4-CONFIG-AND-INSTRUMENT-PLAN.md` went through four
architect × four critic rounds to APPROVED (@799db99) and landed as seven rungs, each committing
alone and green: `49e5630` · `28c3772` · `85b3313` · `c7465bf` · `58687d3` · `cf2d367` · this one.

- **The config field exists.** `boyko_render::OcclusionConfig { mode: OcclusionMode }` — two
  variants, `Off` (default) and `TwoPhase` — a Resource on `HzbConfig`'s surface, read live per
  frame. `BOYKO_VG_OCC_FORCE` and its boot panic left shipping code; the verdict overrides are now
  `boyko_app::OcclusionForce`, a test instrument. **`FORCE_KEEP` is no longer the disarm route;
  `OcclusionMode::Off` is**, and unlike `FORCE_KEEP` it suppresses the split predicate, the late
  passes and the extra descriptor-set bindings.
- **The number exists, and it says NOT RESOLVED.** Ten timestamp brackets in the shipping recorder,
  the piece-3 protocol re-run on that channel: `NetRun +10 240 ns` against a band of `49 152 ns`.
  Every contrast `NOT RESOLVED` — and that is a RESULT, not a failure: the instrument resolves
  per-pass costs, these fixtures do not separate the arms. **The default stays `Off` as a SCOPE
  statement**, not as an inconclusive measurement: default-ON would need `NetRun < −band` across ≥3
  sittings on ≥2 fixtures of differing occlusion density *and* a second consumer for the pyramid, so
  that `HzbBuild` is not charged to this feature alone. P4-6 is one campaign, two fixtures, one
  machine, no second consumer.
- **What is left for the owner is one VALUES call**, recorded in the next item: flipping the default.

The original record follows.

There is no `VG-R3-P4-*.md`. Piece 3's own text assigns piece 4 the **owner-facing config field** —
occlusion culling as a setting rather than an env var — and until then the supported disarm is
`BOYKO_VG_OCC_FORCE=keep`. That is a product-surface decision, not a perf/architecture fork, so it
is not mine to settle.

Piece 4's other inherited job is a number. The three-number measurement returned **NOT RESOLVED on
every contrast**, and the reason is structural rather than statistical: nothing brackets
`vb_batch_cull`, `vb_cull_late` or the late raster scope with timestamps, and `swapchain.rs` sets
`VK_PRESENT_MODE_FIFO_KHR` unconditionally, so wall clock is bounded below by the display refresh
(measured 6.893 ms/frame = 145.1 Hz). The zero control came in at 0.47 % against a resolution band
of 287.91 %. Adding the bracket touches the shipping recorder, which piece 3's boundary excludes —
so it is piece 4's first job if piece 4 wants a number.

---

## 2026-08-07 — VG R3 piece 4 is COMPLETE: two VALUES calls, and the dispositions that close the piece

Rungs P4-1…P4-7 all landed. **Nothing here blocks anything** — the piece ships with the default that
was designed for it. Two items are the owner's to decide, and the rest is recorded so no disposition
is left implied.

### VALUES 1 — should `OcclusionMode` default to `TwoPhase`?

It is one attribute, and a real behaviour change: with piece 4's host disjunct, any world carrying an
`OcclusionCulling` marker would then build a depth pyramid by default.

**My position: no, and it is not an inconclusive measurement.** The decision's failure mode is
DELETED GEOMETRY while its upside is bounded by the early raster's share of a frame — the same
asymmetry that makes the marker itself opt-in. On this corpus the benefit is provably zero (a
converged static scene's late scope correctly draws nothing) and the cost is not. The bar for
flipping it is written down and unmet: `NetRun < −band` across ≥3 sittings on ≥2 fixtures of
differing occlusion density, **and** a second consumer for the pyramid so `HzbBuild` is not charged
to occlusion alone. P4-6 is one campaign, two fixtures, one machine, no second consumer.

### VALUES 2 — present mode is a product surface, and nothing owns it

`present/swapchain.rs` creates the swapchain with `VK_PRESENT_MODE_FIFO_KHR` **unconditionally**, so
every wall-clock measurement in this repository is bounded below by the display refresh, and there is
no owner-facing way to ask for anything else.

Piece 4 deliberately did **not** fix it (disposition (c1)). The reason is not cost: the channel it
would have improved — host wall clock — was superseded by the timestamp brackets and is now labelled
`KNOWN-BLIND`, deciding exactly one thing (*did arming the instrument wreck the frame?*). Present
mode is vsync, tearing and power: an owner-facing `PresentConfig`, and a product decision, not an
occlusion piece's business. **Recorded here as a VALUES item rather than silently carried as a
perf TODO.**

### The dispositions, so none is implied

| item | disposition |
|---|---|
| **(c1)** unconditional FIFO present mode | **OUT** — superseded channel + product surface. VALUES 2 above |
| **(c2)** D8: `vb_indirect_late`'s provenance is covered by nothing | **OUT, BOOKED to framegraph core.** Piece 4 declared no new access, so it neither improved nor worsened it; P4-5 additionally asserts the shipping late chain is field-identical with and without the readback probe. The fix is P2-7's `is_write \|\| res_written \|\| res_seeded` change plus a 14-site audit, whose only gate is a replica this campaign has MEASURED blind to the class it would catch — so that rung needs an instrument before it needs code |
| **(c3)** PROBE-ON barrier-stream rows | **DONE at P4-5**, as a derived delta. Two findings: the plan's re-sourcing prediction is refuted by the tree, and the probe's perturbation is larger than any doc said — nine declared accesses over two passes on seven buffers, eight derived barriers, five pinned barriers moving |
| **(c4)** the intra-pass `TRANSFER → COMPUTE` edge on `VbCullUniform` | **DONE at P4-3, the record-order half only** — and it is a COMPILE-time red in both profiles, where the plan's own shape would have stayed green on the very defect it existed to catch. The DECLARATION half stays open (OQ 9): `FrameGraph::pass_access_count` is private and there is no per-pass accessor |
| **(c5)** the stale future-tense header in `vb_occ_split_gate.rs` | **DONE at P4-7.** It sat in FOUR places, not the two the plan named |
| **(c6)** `goldens/PINS.toml`'s UTF-8 BOM, which strict TOML rejects | **KNOWINGLY LEFT, and the reason is measured.** `golden.ps1 -Bless` writes the file back with `Set-Content -Encoding UTF8`, and the only PowerShell on this box is 5.1, whose `-Encoding UTF8` is BOM-**ful** — verified by round-tripping a BOM-less file through that exact call and getting `EF BB BF` back. Stripping the BOM alone would be silently undone by the next bless; the fix belongs at the WRITER and lands with a bless run. No impact today: `golden.ps1` parses with line regexes, and every strict-TOML check in this campaign strips the BOM explicitly |

### Two gaps piece 4 opened and could not close, recorded rather than absorbed

- **The pin-binary split gate runs PROBE-ON while the pins run PROBE-OFF.** The gap is small and
  named: `vb_probe_dump` is a host-side counter sink that records no commands and cannot enter the
  split predicate. It is still a gap.
- **The dual-read equality invariant is dev-profile only.** A release bench run does not execute it.
  If a release-only divergence between the two query readers is ever suspected, the check has to be
  re-run in the dev profile on the same scene; nothing in the ladder can detect it in release.

---

## 2026-08-07 — RESOLVED — the cull verdict divided, and a division cannot agree with a host oracle

**ANSWERED and implemented in the same session. Kept here in full because the reasoning is the
transferable part, and because the FIRST reading recorded below was wrong in a way worth preserving:
it named a direction from a sample of one.**

**Resolution.** The verdict no longer divides. `for all i: cz_i < occ * cw_i` replaced
`max_i(cz_i/cw_i) < occ` in the shader and in `boyko_render::hzb`'s oracle, `depth_near` moved under
`#ifdef VB_CULL_DEBUG_PROBE` so the shipping module no longer computes the quantity that used to
decide, and the boundary corpus was re-derived to plant against the new predicate. Measured after:

    DepthNearCensus { compared: 72, identical: 72, gpu_below: 0, gpu_above: 0, max_ulps: 0 }
    verdict disagreements: 0 of 72
    24 EXACT-TIE KEEP probes, 24 strict KEEP probes, 24 strict REJECT probes

The tie arm is what proves `<` is strict, and it is now reachable by construction rather than by
luck: the plant uses `z = near·2^k` and `occ = 2^-k`, both dyadic, so the tie is exact on both sides.

**One thing the fix cost, and it is the part worth remembering.** Re-pinning the artifact census
showed `op_ford_less_than` going DOWN by two at the exact step that ADDED a per-corner comparison —
because `!(cz < bound)` lowers to `OpFUnordGreaterThanEqual`, which the census had no field for. A
census that counts only the ordered compare would have read a verdict's *deletion* as a small
decrease and pinned it without comment. The field was added
(`op_funord_greater_than_equal: 4`, two of them the verdict, one per inlined copy).

---

### The finding as originally recorded

**Not a blocker. Recorded because it is a MEASURED correctness finding, and because the fork it
opened was mine to decide — the owner should be able to overrule it before it ships.**

VG R3 piece 3 step P3-4 (the occlusion leaf) is in the working tree, uncommitted. Its new gate
`crates/boyko_app/tests/hzb_verdict_oracle_gate.rs` runs four corpora. Three pass, including the
131,072-pair random corpus and the sentinel corpus. The fourth — exact tangency — fails on its first
probe:

    [64x48 boundary probe 0 (equal)] batch 0: the record's instanceCount is 0 but the oracle
    keeps 1 of 1 instances early. (deferred: gpu 1 / oracle 0)

The GPU **rejects** where the oracle **keeps**. The shader's comparison
(`vb_batch_cull.comp.hlsl:872`) is `return depth_near < occ;` — strict, and correct: equality must
keep. So the operator is right and the VALUE differs — the shader's `depth_near` lands below the
host's. The shader's own comment at `:766-767` named this in advance as *the geometry-deleting
direction*, and the fixture's comment at `:1309-1313` predicted the exact signature: a 1-ULP
disagreement "would show up as a failure on the exactly-equal arm and nowhere else."

**Both predictions were written before the run, and both came true.** The gate is working. This is
the campaign's eighth instance of the pattern — and the first where the instrument caught the defect
instead of being vacuous over it.

### The measurement, and what it overturned

A `-D VB_CULL_DEBUG_PROBE=1` variant now exports the shader's own `depth_near`, level and taps, so
the divergence is OBSERVED rather than inferred. The shipping module is untouched: the
macro-undefined source preprocesses character-identically and `vb_batch_cull_spv_byte_identical`
stays green, so the numbers describe the module that actually ships. Over 72 boundary probes:

    DepthNearCensus { compared: 72, identical: 66, gpu_below: 3, gpu_above: 3, max_ulps: 1, incomparable: 0 }
    verdict disagreements: 2 of 72   (one host=Early gpu=Late, one host=Late gpu=Early)

**This overturns the first reading above.** The divergence is NOT in the geometry-deleting
direction — `gpu_below` and `gpu_above` are 3 and 3, and the two verdict disagreements point
opposite ways. It is symmetric rounding at 1 ULP, not a bias. The first reading came from a single
probe, which is exactly the sample size at which a direction claim is worth nothing.

`level` and all four `taps` are IDENTICAL on every one of the 72 probes, so the window rect and the
level selection already agree exactly and only the depth differs.

### The cause, and why it closes option (A)

Under the corpus matrix, row2 = `[0,0,0,near]` and row3 = `[0,0,1,0]`, so `cz = near` and `cw = z`
are exact and bit-identical on both sides — corroborated by the identical taps. The only inexact
step left is the reciprocal. Vulkan's precision appendix specifies `OpFAdd`/`OpFSub`/`OpFMul` as
correctly rounded but allows **`OpFDiv` 2.5 ULP** at 32-bit; Rust's divide is the IEEE 0.5-ULP one.
`precise` emits `NoContraction`, which constrains contraction and reassociation and says NOTHING
about a division's ULP allowance.

So **(A) as originally posed is dead** — not expensive, *impossible*: no amount of tightening the
existing fold reaches bit-exactness, because the gap is a spec allowance, not a code shape. The
shader comment at `:909-910`, which claims `precise` "forbids substituting a reciprocal-estimate",
is a false claim and is being corrected.

**(B) is also wrong now** that the direction is known: rounding UP would trade one arbitrary
direction for another, and the 1-ULP bound it would lean on is measured on 72 probes on ONE device
while the spec permits 2.5.

### What is being done instead: remove the division from the DECISION

For `cw_i > 0` — already guarded by the behind-eye early-out —

    max_i (cz_i / cw_i) < occ    <=>    for all i:  cz_i  <  occ * cw_i

The right-hand form is one correctly-rounded multiply under `NoContraction`, so the shader and the
oracle agree **by construction** rather than within a tolerance, and it is *cheaper* than the divide.
That is an exact reformulation, not a relaxation and not a bias — which is why it is preferred over
every option originally listed. The window rect keeps its divide, which is measured to agree exactly.

**(C) — declare tangency untestable and relax the arm — stays rejected**, and is now unnecessary.

---

## 2026-08-07 — Profiling + logging, review round 3: both REJECTED, and the SEAM is incompatible

The two plans reached revision 3 and were reviewed a third time — separately, and for the first
time **against each other**. Verdicts: profiling `REJECTED (6 blockers)`, logging
`REJECTED (10 blockers)`, seam `INCOMPATIBLE AS WRITTEN (6 blockers)`. Revision 4 is in flight;
these three items are not the reviewers' to decide.

### The seam was never designed, and that is the round's main finding

Two prior rounds read one plan each. The first reader of the seam found the two documents
asserting **contradictory facts**: profiling justifies moving its ABI into `boyko_utils` because
that crate has zero dependencies; logging states flatly that `boyko_utils` depends on `boyko_log`.
Both cannot hold. Below that, each plan independently invented the same four primitives — a
per-thread lane index, an `rdtsc` calibration, a never-freeing lane allocator, and a loss
accounting — with incompatible semantics: one worker would be lane 5 to the profiler and lane 37
to the logger, and only one of the two clocks would know about a suspend/resume. That is precisely
the failure Principle 0 names: a capability two subsystems need, built twice as per-crate adapters
instead of once as a kernel feature.

**Decided by me, not the owner** (architecture, per the standing agreement): a new zero-dependency
bottom crate `boyko_diag` owns the clock, the lane registry, the loss vocabulary and the
never-freed storage policy; it is *diagnostically mute* (it emits no `boyko-####` code and prints
nothing, which is what keeps the graph acyclic); `profiling_abi` is hosted there rather than in
`boyko_utils`, which keeps its empty `[dependencies]`. Full design:
`docs/DIAGNOSTICS-SUBSTRATE-PLAN.md`.

### VALUES 1 — how much does a SHIPPED title pay for diagnostics?

Nobody had computed the joint number. Measured from the two plans' own tables:

| | profiling alone | logging alone | **jointly** |
|---|---|---|---|
| dev, `.bss` + reserved | 6.65 MiB | 3.46 MiB | **9.33 MiB** (10.11 naive; the shared crate saves 0.78) |
| **shipping** | 0.85 MiB | 1.16 MiB | **1.95 MiB** — **WRONG; corrected immediately below** |
| hot-path cache lines | 3-4 | ≤ 4 | **7-8** |

> **CORRECTED 2026-08-08 — the shipping figure above was already wrong on the day it was first put
> to you, and it is corrected in the open here rather than quietly re-based.**
>
> **1.95 MiB has never equalled the sum of its own operands, in any revision.**
>
> - As put to you above (rev 3): `0.85 + 1.16 = **2.01**`, printed as **1.95**.
> - At the corpus's first carved revision the operands moved and the total did not:
>   `0.89 | 1.15 | naive 1.95`, and `0.89 + 1.15 = **2.04**`.
> - **Then a second, independent error surfaced underneath the first.** The logger re-derived its
>   own `shipping` column term by term (`docs/diagnostics/logging/01-EMISSION-RING.md:130`:
>   512 + 32 + 16 + 16 + 4.25 + 0.008 + 256 + 64 + 320 = **1 220.26 KiB ≈ 1.19 MiB**) and showed
>   that **no subset of its rows sums to the 1 180 KiB** the seam was quoting — so 1.15 was not a
>   different configuration, it was wrong too.
>
> There is no third quantity 1.95 could have been. Both revisions state that the shared substrate
> saves **ZERO bytes in shipping** — the 0.78 MiB saving is dev-only — and with a zero saving the
> joint figure simply **is** the naive sum. **The corrected shipping figure is ≈ 2.08 MiB**
> (908 + 1 220.26 = 2 128.26 KiB), against ≈ 2.01 MiB on the numbers as they were handed to you.
> The error ran against you every time: the ask was understated by 0.06 MiB then and by 0.13 MiB
> now.
>
> 🔑 **The lesson that outlives the number, because it defeated a repair pass whose stated job was
> to catch exactly this.** After the first correction the seam table was *internally consistent* —
> `0.89 + 1.15` really is `2.04` — and that is precisely why the stale **operand** survived. **A
> total that checks out against its printed operands proves nothing about those operands.** The
> durable rule: with a zero shipping saving the joint figure is the sum of the two columns, and
> any edit to it must re-read the source rows it quotes rather than re-adding the numbers already
> printed beside it.
>
> **What the figure MEANS also changed, and that narrows what is being asked.** S13 —
> *free when not enabled*, folded in after this entry was written — moved every syscall, thread,
> hook and first write off the boot path and onto the enable path, so a shipped process that never
> enables diagnostics **never touches these tables at all**. An untouched all-zero `.bss` table is
> emitted by the linker with a virtual size and no raw data, so ≈ 2.08 MiB is **declared address
> space, not resident RAM** (`docs/diagnostics/SEAM.md` §S13, MEMORY row). Two limits on that,
> both stated by the corpus itself rather than smoothed away: the property holds **only if boot
> touches nothing** — one write to one lane buffer commits that page and it is lost for that page
> — and the corpus **explicitly refuses to claim** that the loader leaves an untouched page
> uncommitted (`substrate/section-report` proves the bytes are absent from the *image*, and no
> more; `docs/diagnostics/substrate/05-LADDER-GATES.md`, gate DG12).
>
> **So the question is narrower than this section's heading suggests.** Not *"what does a player's
> machine spend on diagnostics"*, but: **is ≈ 2.08 MiB of declared address space — resident only
> in the sessions where diagnostics are actually switched on — an acceptable price for a shipped
> title?** Still a VALUES call, and still not mine.

So the profiling plan's headline **"≤ 1 MiB retail" is false in the configuration that will
actually ship**, and the shared substrate saves **nothing** in shipping — its 0.78 MiB saving is
dev-only. It is bought for correctness (one lane number, one clock epoch, a loss report that
cannot itself be dropped), not for footprint, and neither plan may claim otherwise.

Cutting **≈ 2.08 MiB** means cutting one of: logging's 32 × 16 KiB lanes (512 KiB), `SINK_OUT`
(256 KiB), or the profiler's dynamic-zone arenas (96 KiB — *the current revision states this third
candidate as **40 KiB** in `shipping`, `docs/diagnostics/SEAM.md` §Open — needs the OWNER, item 1;
the divergence is recorded here, not resolved, because resolving it belongs to the profiling
plan*). **This is a VALUES call about what a player's machine spends on diagnostics, and it is not
mine.**

### SCOPE 1 — what does `shipping-min` actually mean?

Logging's `shipping-min` exists for a title that wants **no resident diagnostics thread**. But
profiling's `Always` tier still writes a telemetry window synchronously on the dispatcher, so such
a title pays a periodic `write_all` anyway. Either `shipping-min` also disables telemetry, or the
profile does not mean what its name says. **SCOPE call.**

### SCOPE 2 — the plans are growing faster than they are converging

Three review rounds, and the blocker count has not come down: **35 findings → 17 → 22**. The two
documents are now 3370 lines for two subsystems that do not exist as a single line of code, and
more than half of round 3's new blockers were introduced by what round 2 added — the game-facing
half — while the seam only became visible because both documents grew into full architectures.

That is a signal about **how much is being designed at once**, not about the reviewers. The
alternative is a narrowed first tranche — `boyko_diag` + CPU zones + log levels — built and
measured, with telemetry, retention and the game-facing API returning afterwards on a working
foundation. Stated here as an option; **the owner decides the scope, not me.** Work continues on
the full revision 4 unless told otherwise.

### SCOPE 3 — the workspace's `--cfg loom` leg has not compiled, and nothing said so

Found at rung D1, 2026-08-08, while checking that the new `boyko_threadpool -> boyko_diag` edge
did not disturb the loom build. It did not. The loom build was **already broken**:

```
RUSTFLAGS=--cfg loom cargo check -p boyko-threadpool --lib
error[E0599]: no method named `get_mut` found for struct `loom::sync::atomic::AtomicPtr<T>`
  --> crates/boyko_threadpool/src/scope.rs:185:39
```

loom 0.7.2 offers `with_mut`, not `get_mut`. `-p boyko-ecs` fails on the **same** error because it
reaches the same lib, so **both** crates that carry a `[target.'cfg(loom)'.dependencies]` block are
dead, not one. `rg loom .github/workflows` returns nothing — no CI leg passes `--cfg loom`, which
is why this has been invisible. Confirmed not to be a D1 regression: `git stash`-ing the D1 diff
reproduces the identical error at `93dbcf8`.

Why it needs a decision rather than a quiet fix:

1. The substrate plan cites these two crates as the working precedent `boyko_diag`'s `claim_lane`
   loom model will copy. The **manifest** shape is a valid precedent; the claim that a model *runs*
   beside it is not. The plan text is now qualified — the citation is not silently left standing.
2. The fix at `scope.rs:185` is one line inside an `unsafe` `Drop` on the scope-teardown path, and
   a green `cargo check` under `--cfg loom` is **not** a run model. Making the leg mean something
   means running the models, and this machine crashes loom **release** binaries at startup
   (recorded previously, unrelated).
3. So the real question is scope: (a) repair `scope.rs:185`, run the existing models in debug, and
   add a CI leg that keeps them compiling; (b) repair it and add no CI leg, accepting the same
   silent rot later; or (c) leave it, and land `boyko_diag`'s concurrency evidence as the
   proptest + Miri legs only, stating in the plan that no loom model backs `claim_lane`.

**Not repaired at D1** — it is outside D1's subject and the choice above is not mine. The lane
claim path's Miri and property legs are unaffected and still planned.

---

## KNOWN FRICTIONS — no decision needed, recorded so they are not rediscovered

- **`target/` grows without bound and silently breaks builds.** It reached 73 GB and hit zero free
  space mid-build this session. The failure presents as a *mingw linker error*; the real cause is on
  the last line (`no space on device`). `cargo clean` recovered 73.6 GiB.
- **The trybuild test fails under a concurrent full-suite run** (`compile_fail_frame_write_token`,
  3/3 fixtures) and passes standalone. trybuild spawns its own cargo into the same `target`. Not a
  flake — reproducible contention.
- **`cargo test` STOPS at the first failing binary, and the suite count silently shrinks.** A run
  that trips the flake above reports ~51 suites instead of ~445 — so "I ran the full suite" can mean
  "I ran a ninth of it" with nothing in the output saying so. Always pass `--no-fail-fast` when the
  claim being made is about coverage, and read the suite COUNT, not just the failure count.
- **graphify has been off-target for the render/VB path** for this entire session; every query
  returned `boyko_demo` internals. Grep/Read is the working path there.
- **The `graphify` binary is not installed in this environment at all** (not on `PATH`), while the
  `PreToolUse` hooks demand `graphify query` before every Read and Grep. `graphify-out/` exists but
  its newest subdirectory is from July, so the graph is also stale. The hooks only remind, never
  block — but they fire on every file access. Fix is either an install or a hook that checks the
  binary exists before demanding it.
- **The ECS's global query-type registry can exhaust under the full lib suite.**
  `MAX_QUERY_TYPES = 1024` is a process-global cap minted lazily, and `boyko-ecs --lib` runs 864
  tests in parallel. When scheduling happens to mint the 1025th distinct query shape, whichever test
  is unlucky dies with a TERMINAL panic naming the cap. Observed once, then 3 consecutive clean runs
  of the same binary. It is order-dependent, not a regression signal — check by re-running before
  bisecting anything.
- **`boyko-engine --test internal_docs_anchors` is RED on `master`'s content, and has been for a
  while.** 13 stale line anchors (10 in `docs/MESHLET-VIRTUAL-GEOMETRY-PLAN.md`, 3 in
  `docs/SYSTEMS.md`) plus 8 over-waivers against a cap of 6 in the meshlet plan. **Proved
  pre-existing at L5** by stashing the whole rung and re-running: byte-identical failure. It makes
  `cargo test --workspace` red for everyone, so the next person to run it will spend the same
  fifteen minutes proving it is not theirs — which is the cost of leaving a standing red in place.
  Re-deriving the anchors is a self-contained chore and belongs in its own commit.
- **A line inserted into a widely-cited source silently invalidates every doc anchor below it.**
  L5 added `VmColumn::as_mut_slice` at ~line 355 and shifted `vm_column.rs:437-449` to `462-474` in
  two corpus files. The anchor test does not cover `docs/diagnostics/**`, so nothing would have
  said so. Grep `<file>.rs:[0-9]` across `docs/` after any insertion into a kernel primitive.
- **A gate can be RED for five commits because nobody runs it, and no gate can close that.**
  `G2a`'s file census (`tests/gpu_blocking_reader_census.rs`) went red the instant rung 5a landed
  `present/gpu_zone.rs`, whose module doc names `vkGetQueryPoolResults` while explaining what the
  BLOCK cost. It stayed red across `ee9196b6`, `cb54752d`, `8ca4e05b`, `cf8ffd20`, `7ae9162a` —
  three of which reported "workspace green". Found at 5c only because a full `--workspace` run
  finally happened. The mechanism worked perfectly; the *asking* was the gap. Practical rule:
  **a rung that adds a file must run the census gates, not only its own.**
- **The disk fills to zero and it looks like a mingw linker bug.** Second occurrence (see the
  2026-07-23 audit note). `target/` reached 72 GB with 8 KB free on `D:`, and the symptom was a wall
  of `linking with x86_64-w64-mingw32-gcc failed` across a dozen unrelated targets. Cure:
  `rm -rf target/debug/incremental` (12 GB here). Check `df -h /d` BEFORE reading a linker error.
- **G10's A/B runs as two processes and the corpus says one; the reason is in the docs but it is a
  deviation an auditor should see.** Leg A's `read_vb_bench_ns` waits with `VK_QUERY_RESULT_WAIT_BIT`,
  so a single process alternating legs would reach it on a frame leg B recorded — the hang class
  P4-1 removed. If a future rung wants the corpus's literal shape, it must first make leg A's
  readback non-blocking, which is rung 7's deletion anyway.
- **RESOLVED (owner: "whichever is more performant") — profiling rung 6's G10 fork went to the host
  arming knob.** `BOYKO_GBUF_BENCH` costs one boot-time `Option` that is `None` in every shipped run
  and expires with rung 7; the alternative would have forced `GpuZoneRecorder::open_frame` and
  `retire` to `&self`, deleting clause (c) of `FrameSlot`'s `Sync` argument permanently and pushing
  `set_mark` toward a locked RMW in a hot recorder. **The leg is armed and never read** — the witness
  clause needs no timings, and `read_query_pool_ns` would hang on a frame that skipped a pass.
  ⚠️ **Rung 8's reader must consult the witness masks before it waits on anything**: three of the
  four software-ray passes are bracketed inside their own `if let` arms, and neither old gbuffer
  collector has a totality epilogue.
- **RESOLVED (owner: "decide yourself what is optimal") — profiling rung 7's artifact format is
  decided and its writer/reader/gate have shipped.** See `profiling/artifact.rs`'s module doc for
  each decision and what it costs to get wrong. ⚠️ **One of the six answers turned out to be wrong in
  its REASONING and was corrected by insisting on a RED**: the fear that a wider file collapses
  `vg_occ_split_timing.rs:916`'s GCD is false — that consumer's own `(v * 10.0).round()` absorbs the
  extra digits, measured across seven values. One decimal is still right, for smaller reasons
  (direct comparability with the printed lines, which is what makes the next step's A/B possible).
  The six values, as decided:
  1. **Numeric precision in the artifact.** `vg_occ_split_timing.rs:916` reconstructs the GPU tick
     lattice by GCD over **tenths**, because that is the precision the summary prints. Full-precision
     `f64` collapses the GCD and sub-floors every band; the file's own doc measures the error at
     **32×** and says such a choice *"would satisfy every assertion here while under-stating the
     instrument's resolution by the whole lattice factor"*. **A silent false-win, not a red test.**
  2. **File path, per-run uniqueness, truncation.** `append_artifact(p, path)` takes a caller path
     and nothing else — no default, no rotation, no env knob. `vg_decidability_floor.rs` spawns 42
     sequential children; a fixed path is a stale-read generator.
  3. **One file = one sitting, one process, or many appended runs.** `G24`'s reverse RED is *defined*
     on staleness and cannot be written until this is chosen.
  4. **Who aggregates the 21 per-session artifacts into `docs/PROFILING-FLOOR.md`** — rung 7b
     depends on it and no line assigns it.
  5. **Whether `WorkloadTag` is an artifact field.** `resolve` checks it, 7b publishes it into
     markdown, nothing says the session file carries it.
  6. **What the artifact records when the device declines timestamps** — today an `eprintln!` that
     three consumers key their third outcome on.
  ⚠️ **`G24`'s reverse RED named two fields that cannot do the job, and one of them does not exist.**
  `crates/boyko_diag/` has **no `build.rs`** and `BUILD_HASH` appears nowhere in the workspace — a
  planned rung-0 artifact that never landed. `SessionId` exists but is minted INSIDE the child, so a
  parent cannot predict it. The discriminator is therefore a **parent-supplied run token**, the only
  field that can catch staleness within one run.
  Rung 7's remaining halves, in order: the reducer that fills the artifact, the producer wiring
  (verified by A/B against the still-printing channel while BOTH are live), 7b's floor
  re-measurement, then the deletions — **713** lines from `gpu_timing.rs`, **1381** from `runner.rs`
  (31 % of the file), **465** from `gpu_scene/mod.rs`, plus five consumer migrations.
- **RESOLVED (owner: the STRICT option of three) — the workload tag is two halves, and an
  undeclared one is not a floor.** Measured while opening rung 7's consumer migrations. The tag is
  `format!("{path:?}_{legs:?}")` over `ResolvedRenderPath` — `deferred_both`, `visibilitybuffer_mesh`
  and so on. `vg_decidability_floor.rs` runs its NULL experiment twice per repetition, once with
  `BOYKO_VB_FROXEL_FORCE_OFF` and once without, and **neither `path` nor `legs` changes between
  them**: `froxel_light_cull` is a separate field of the same struct and the tag does not read it.
  `BOYKO_VB_BENCH_LIGHTS` (`N_ps`) is invisible to the engine entirely.
  The migration itself is not blocked — the floor test writes each leg to its own file path and
  never reads the tag to separate them. What the hole costs is **downstream**: `resolve` refuses a
  `Floor` whose `workload` differs, and that refusal is the ONLY mechanism keeping a floor measured
  on one configuration from bounding a delta measured on another. With this tag, a flat-leg floor
  silently bounds a froxel-leg claim — which `vg_decidability_floor.rs`'s own "What this does NOT
  decide" forbids in words (*"It is one CONFIGURATION"*, *"a rung that measures at a different scale
  must re-measure its own floor rather than cite this one"*).
  ⚠️ **And a correction to this entry's own first sentence about the mechanism.** It said `resolve`
  refuses a mismatched `Floor` as if that were shipped code. MEASURED: `Floor`, `resolve`,
  `FloorWorkloadMismatch` and `NotResolved` exist **only in the corpus documents** — `rg` over
  `crates/` returns nothing. They are rung 8's content and are unwritten. So nothing was silently
  wrong; the tag is the INPUT to a comparator that does not exist yet, which is why what it names
  had to be settled before 7b publishes a floor that later rungs cite.
  **Shipped as decided:**
  * **Derived** — [`config_tag`] over the WHOLE `ResolvedRenderPath` (readable `path_legs` prefix +
    8 hex of FNV-1a over every field), not a hand-picked subset: the bug was not that the wrong
    field was chosen, it was that fields were chosen. A field added to that struct invalidates prior
    floors deliberately — floors on this box drift faster than that anyway.
  * **Declared** — `content_tag`, from `BOYKO_PROFILE_WORKLOAD`, set by the measuring test in its
    own spawner code where the value already lives, not in an operator's shell.
  * **The refusal is enforced NOW, not promised to rung 8**: `Artifact::floor_source` returns
    `UndeclaredContent` on an empty or blank content tag, because a clause whose subject does not
    exist yet is a promise rather than a gate. A missing KEY is a malformed header, kept distinct
    from a present-but-empty one — the whole of the refusal is that distinction.
  RED run: revert the derivation to `path × legs` ⇒ *"the flat and froxel legs produced the SAME
  workload tag ... flat: deferred_both, froxel: deferred_both"*. Measured on a live run:
  `workload_tag = "deferred_both#99f4482e"`.
  ⚠️ **Narrowed 2026-08-10, and the obvious cause is ruled OUT.** The module already serialises
  itself: `armed()` takes `test_serial()` and hands the `MutexGuard` back to the caller, so every
  test that arms a store holds the module's one lock, and a second explicit site covers the
  plugin test. So the contention is **not** profiling test against profiling test. What remains is
  state global to the PROCESS rather than to the module — `boyko_diag::profiling_abi`'s zone
  `REGISTRY`/`NEXT_SLOT` (slots are minted lazily by `declare_zone!` and never returned) and the
  store's `bind_world` — which any of the other ~880 tests in the same binary can move. Both flakes
  pass 915/915 in isolation and in repeated full-lib runs; they fail only under the full workspace
  sweep. **Next step is to identify which non-profiling test touches the registry**, not to widen
  the module's lock, which is already as wide as the module.
- **`boyko-ecs --lib`'s profiling tests are ORDER-DEPENDENT FLAKES — now TWO of them.**
  `every_dispatching_round_records_one_span_and_one_width` joined
  `one_zone_taking_a_hundred_thousand_samples_keeps_count_exact` on 2026-08-10: observed failing
  once in a full `--workspace --all-targets` run, then passing in isolation and in two consecutive
  full lib runs (915/915 each). **A second name in the same class raises the priority**: this is
  no longer one unlucky test but a property of the module, and a real regression here would be
  indistinguishable from the flake. Original entry follows. Observed failing once in a full `--workspace --all-targets` run, then
  passing in isolation and in two consecutive full lib runs (915/915). Same class as the
  `MAX_QUERY_TYPES` note above: process-global profiling state (lanes, zone slots) against 915
  tests in parallel. Re-run before bisecting.
- **⚠️ A KNOWN-RED TARGET IS A SHADOW — the workspace gate must run `--no-fail-fast`.**
  MEASURED 2026-08-10. `cargo test --workspace --all-targets` **stops at the first failing target**,
  so `internal_docs_anchors` — red for everyone, long known — had been hiding every target ordered
  behind it. Every "workspace suite green except the pre-existing anchors failure" reported during
  the diagnostics campaign was a claim about **what cargo reached**, not about the workspace. Run
  with the flag, there were **three** red targets, both of the others older than the session that
  found them:
  * `boyko_rhi_vulkan --test compile_fail_frame_write_token` — `26b385eb` (2026-08-06) added one
    line to `token_use_after_submit_rejected.rs` and never re-blessed its `.stderr`; the entire diff
    was `41:5` vs `42:5`. **Red for the 87 commits since.** FIXED by hand-editing the two line
    numbers rather than `TRYBUILD=overwrite`, because that fixture's own comment warns that blessing
    can turn a right-for-the-wrong-reason red into a wrong green — the error kind, the moved value
    and the move site were all verified unchanged first.
  * `boyko_rhi_vulkan --test cluster_bound_arraylength` — VG R3's `vb_batch_cull` (+ its `-D DEBUG`
    sibling) gained an `OpArrayLength` and `BOUND_BY_ARRAYLENGTH` was not updated in that commit,
    which is what the gate's own message instructs. FIXED, and the pin **widened to carry each
    entry's SUBJECT**: the set no longer has one, since those two bound `VbLateCount`'s reserved
    tail slot rather than a froxel light walk, and the old failure text would have given a reader
    advice about a `use_clusters` guard those shaders do not have.
  * `boyko-ui --test p3_watch_zero_alloc` — `watch_nochange_path_is_tiny_and_far_below_reload`
    reported `nochange 1, reload 1`: the "reconciling" tick was reporting the no-change path's cost
    because it WAS the no-change path. The watcher's signature is `(mtime, size)` and the fixture
    rewrote `Px(77)` over `Px(40)` — **the same byte length** — so detection rested entirely on the
    filesystem clock, and this box's mtime granularity swallows the fixture's 3 ms sleep. `4956420c`
    had already diagnosed this class once ("the hot-reload flake was the FILESYSTEM CLOCK, not
    shared state") and answered it with longer sleeps, which buys margin against a granularity
    nobody measured. FIXED by making the rewrite change the SIZE, which removes the dependence
    instead of widening it: `nochange=1 reload=53` after.
  ⚠️ **And my own reporting of this finding was truncated the first time.** The first sweep's
  failure list was piped through `head -10` and I read the truncation as the total — "three red
  targets" when the honest count was six. The same mistake one level up from the one being
  reported. The full picture, measured:
  * **Genuinely red, now FIXED**: `compile_fail_frame_write_token`, `cluster_bound_arraylength`,
    `p3_watch_zero_alloc`.
  * **Genuinely red, NOT fixed**: `internal_docs_anchors` — 25 stale anchors across three internal
    docs plus an over-waiver count above its cap. Pre-existing, unrelated to any campaign here, and
    large enough to be its own unit of work.
  * **Not red at all, but FAILING UNDER THE FULL PARALLEL SWEEP**: `boyko-log --lib` (84/85 in the
    sweep, **85/85** in isolation), `boyko_rhi_vulkan --test sdf_gbuffer_hybrid` (**43/43** in
    isolation, 54 s), and the two `boyko-ecs --lib` profiling tests recorded above. Four targets in
    one class. **The workspace suite is therefore not deterministic**, and a real regression in any
    of them would be indistinguishable from the noise. Every one touches process-global or
    device-global state — profiling lanes and zone slots, logging sinks, the GPU device — which is
    what a `--test-threads` bound or a per-target serial marker would address.
  `CLAUDE.md`'s build-command block now carries the flag and the reason.
- **`.claude/settings.local.json` is dirty** from earlier sessions and is deliberately never staged.

## Rung 9 — `resolve`'s session check refuses every real leg pair, and reports it as `EpochBreak`

**Found while wiring rung 9's correlation, which gives `clock_epoch` a real meaning in this tree for
the first time. Not introduced by rung 9 — surfaced by it.**

`crates/boyko_app/src/profiling/contrast.rs`'s `LegSummary` carried a field named `clock_epoch`
holding `(header.session_lo, header.session_hi)` — a `SessionId`, not
`boyko_diag::clock::clock_epoch()`. Rung 9 renamed it to `session`, because the artifact now carries
a real `cpu_gpu_epoch` and two different things would otherwise have shared one name in one module.
The rename is done. **Two things about the check it feeds are not, and both are the owner's call:**

1. **The refusal reports `NotResolvedReason::EpochBreak` for a SESSION difference.** That is
   defensible in spirit — `clock_epoch()` is a per-process counter, so "both at epoch 0" from two
   processes compares two numbers that mean nothing to each other — but the reason word names
   something the check does not test. `G13`'s sibling clause pins the word `EpochBreak`, so
   renaming it is corpus surface, not a local edit.

2. ⚠️ **On real inputs the check refuses unconditionally.** MEASURED: `resolve` and
   `LegSummary::from_artifact` have **no caller outside `contrast.rs`'s own tests** — rung 8 shipped
   the comparator to *license* later verdicts, not to serve one — and every leg pair a real harness
   would build today comes from two spawned child processes (`vg_decidability_floor.rs`'s protocol
   is seven processes per condition). Two processes have two session ids by construction, so the
   first production consumer of `resolve` will find that it returns `NotResolved { EpochBreak }` for
   every pair it is ever given. The existing tests do not catch this because both legs are hand-set
   to the same value.

**What a fix would have to decide** (not decided here): whether cross-process legs are comparable at
all. If they are — and the whole floor protocol assumes so, since it pools seven sessions — then the
check is wrong as written and the real guard is something else (same `workload_tag`, same box, same
`clock_epoch` *within* each artifact). If they are not, then the floor protocol and this check
contradict each other and one of them is the error.

## Rung 9 — the per-frame ring is now TWO deferrals pointing at one mechanism

The correlation is published once per window with a measured 173 ppm drift across it
(`Correlated::deviation_at_ns` interpolates). Rung 8's per-zone `vkCmd*` counters reach the printed
census but not the artifact, for a structural reason of the same shape (the witness resets each
frame while `retire` yields a frame recorded ~4 frames earlier). Both want a per-frame channel that
does not reduce to medians — which is also what the owner's original ask needs ("break a frame down
by system and pass, catch per-frame spikes"). Recorded so it is built once rather than twice.

## The `hwrt` feature leg does not COMPILE — pre-existing, found by rung 9's clippy sweep

**MEASURED, and proved not to be this campaign's:** `cargo clippy -p boyko-app --lib --features
boyko_rhi_vulkan/hwrt` fails with

```
error[E0063]: missing fields `atrous_layout_denoise_hwrt`, `motion_cam_ubo_ring`, `mv_bind_group`
and 21 other fields in initializer of `boyko_rhi_vulkan::present::GBufferScene<'_>`
    --> crates\boyko_app\src\gpu_scene\mod.rs:6298:25
```

Twenty-four missing fields. Confirmed pre-existing by `git stash`ing every rung-9 source change and
re-running: **identical error on the untouched tree at `71085737`.**

**Why nothing caught it.** `hwrt` is `default = false`, and every gate in this tree —
`cargo check --workspace --all-targets`, the clippy gate, the test sweep — runs the DEFAULT feature
set. The leg is never built, so it can rot without turning anything red. This is the same shape as
the two findings already recorded above (a target ordered behind a known-red one; a virtual manifest
type-checking a subset): **a configuration nothing builds is a configuration nothing gates.** Rung 8
recorded that lesson for `profiling-alloc` and fixed it by making both configurations buildable;
`hwrt` is the larger instance of it and has been un-built for long enough that the drift is
twenty-four fields wide.

**Not fixed here.** It is unrelated to rung 9, it needs `GBufferScene`'s twenty-four `hwrt` fields
understood one at a time, and guessing at them would be worse than the current honest break. **Owner
call:** repair the leg and add it to the gate set, or state that `hwrt` is dormant and stop
implying otherwise.

## Rung 10 — two corpus gates could not run as written, for two different reasons — **RESOLVED 2026-08-10, see the top of this file**

Both are recorded in `docs/diagnostics/profiling/05-LADDER-GATES.md`'s rung-10 record with their
arithmetic. Surfaced here because each is a **decision the owner may want to take differently**, not
merely a note.

**1. `G17`'s absolute nanosecond thresholds were replaced by an A/B ratio.** The row asks for five ns
budgets in one sitting, the tightest pair being "static-armed ≤ 12 ns" against "dyn-armed ≤ 14 ns" —
two nanoseconds of headroom. This campaign measured its own artifact-channel floor at **6.5 %**,
with repetitions spanning 4.7–14.3 %, on GPU passes costing microseconds. A 2 ns budget is inside
that noise by an order of magnitude. What ships instead implements **both** variants — the shipped
gate and the `REGISTRY[id]`-dereferencing one the row names as its RED — and interleaves them in one
process, asserting the shipped one is not slower. MEASURED (debug): 10.89 vs 22.01 ns/iter, 2.02×.
**If the owner wants the absolute thresholds, they need a release-profile bench harness and a
recorded per-box floor for the ns scale** — neither exists, and inventing a threshold without one is
how a gate comes to fail for a background process.

**2. `G22b` clause 2 names a symbol that does not exist and a failure that cannot be written.** The
clause says *"a `#[test]` declaring a `.bss` array sized from a `ProfilerConfig` value must fail
`assert_bss_eligible` at compile time; remove the const-assert ⇒ it compiles ⇒ red."* MEASURED:
`assert_bss_eligible` has **zero hits** in `crates/` (the symbol is `assert_zero_init_eligible`), and
the failure it describes is impossible here — `SyncCells<T, N>` takes its extent as a **const
generic**, so a run-time `ProfilerConfig` value cannot size one whatever any assertion says. There
is no const-assert to remove because nothing needs one. A `trybuild` case gating the property rung 10
DID introduce ships instead (deleting `DYN_DESCS`'s `MaybeUninit` wrapper must be `E0277`). **Owner
call: rewrite the clause against the real symbol, or delete it as satisfied by the type system.**

**And `G22b` clause 1 remains BLOCKED on the same missing tool as `G22a`** — no
`llvm-readobj`/`objdump`/`nm` under the active `stable-x86_64-pc-windows-gnu` toolchain. Rung 10
added two more symbols (`DYN_DESCS`, `DYN_NAMES`) to the set that probe must cover when
`rustup component add llvm-tools` lands, so the D0 line item now unblocks four names rather than two.

## Rung 10 — `G23b`'s literal RED is not producible, and no setting of the constant makes it so

The row's RED: raise `MAX_USER_BUDGET` in the **shipping** profile from 512 to 3072 ⇒ +20 KiB
`REGISTRY` and +120 KiB `DYN_DESCS` ⇒ 1 348.2 KiB crosses the 1 280 KiB budget.

There is no shipping profile: the `BOYKO_PROFILE` axis is rung 14, and MEASURED, **no `build.rs`
exists anywhere in this workspace**. So the residency gate runs at the dev row against a **16 MiB**
budget. And the constant has a hard ceiling that is not a policy choice: zone ids are `u16`, so
`ENGINE_ZONE_SLOTS + MAX_USER_BUDGET` must stay under `u16::MAX`, capping `MAX_USER_BUDGET` at
~61 439. At 8 B/id in `REGISTRY` plus 24 B/id in `DYN_DESCS` that is **~1.9 MiB of growth against a
16 MiB budget** — it does not cross, and nothing a caller can set makes it cross.

An upper bound nothing can push past is a gate that cannot fail. What ships is a **composition
identity** — domain 3 must equal the four `.bss` terms, computed independently — whose RED *is*
producible: dropping `dyn_descs_bytes()` from the sum gives `left: 143360, right: 217088`. That is
the claim the row's own title makes (*"this row is what puts their bytes inside the budget sum"*),
and it is gated. **The budget clause itself stays honest but toothless until rung 14 gives it a
shipping row to be tight against.**

---

## Rung 13 — a second CRC-32 table, and the graph edge that would remove it

**Recorded rather than decided, because both options cost something real and neither is urgent.**

`boyko_diag::telemetry` computes the block CRC with its own 256-entry const table (1 KiB of
`.rodata`). `boyko_image::png` already has one — same polynomial, IEEE 802.3, private, and shaped
for PNG chunks (`crc32_chunk` takes the chunk kind and prepends it).

The duplication cannot be removed by using `boyko_image`'s: `boyko_diag` must keep an **empty
`[dependencies]`**, which is the property that makes it the bottom of the graph. The only direction
that works is the other one — hoist a shared CRC-32 *into* `boyko_diag` and have `boyko_image`
depend on it. That is refused here for now, on the crate's own rule: a checksum is not a diagnostics
primitive, and §4's growth checklist admits a module only when **both** subsystems write it and a
disagreement between two copies would be observable in a joined artifact. Two CRCs over two
different byte streams cannot disagree with each other about anything.

**Cost of leaving it:** 1 KiB of `.rodata` and six lines, twice.
**Cost of hoisting it:** the bottom crate gains a general-purpose utility, `boyko_image` gains an
edge into the diagnostics substrate, and the growth rule loses the property that makes it hard to
satisfy.

Owner's call if the second one is ever preferred. Nothing is blocked either way.

## Rung 13 — `G26`'s budget is a RELEASE claim, and the gate says so instead of pretending

`G26`'s figures (`__telemetry_reduce` p95 ≤ 150 µs, `__telemetry_write` ≤ 200 µs, sum ≤ 350 µs) are
asserted only under `not(debug_assertions)`. MEASURED, this box, 64 quantile zones, p95 over 32
runs:

| | debug | release |
|---|---|---|
| `__telemetry_reduce` | 5 820.8 µs | **128.0 µs** |
| `__telemetry_write` | 163.1 µs | **11.2 µs** |
| **sum** | 5 983.9 µs | **139.2 µs** |

A debug build is **43× over** the total budget. Asserting the budget there would red on every
developer's machine and prove nothing about the shipped one, so what the gate asserts in *every*
profile is the property the budget encodes and a profile cannot change: the reduce dominates, and it
is the term that scales with the quantile count.

~~**The open half:** the release leg is not in CI today. `scripts/` has no release test step, and the
five-profile CI matrix is rung 14's content. Until rung 14 lands, **the budget clause runs only when
somebody runs `cargo test --release`**, and this note is the record that it is not automatic. It is
the same shape as rung 10's `G17` and is expected to be resolved by the same rung.~~

**CORRECTED at rung 14, 2026-08-11: THAT PARAGRAPH WAS FALSE WHEN IT WAS WRITTEN.**
`.github/workflows/ci.yml`'s `test` job is a `matrix: profile: [debug, release]`, and its release
arm has run `cargo test --workspace --all-targets --release` since long before rung 13. A second
job, `force-alloc-panic`, runs the release suite again under `--cfg force_alloc_panic`. Neither
excludes `boyko_app`, so `G26`'s budget clause has been running in CI on every push the whole time.

The defect is not the conclusion, it is **where I looked**: I checked `scripts/` for a release step,
found none, and reasoned from that to a claim about CI — without opening the CI file. That is the
root cause this corpus has already written down in as many words: *verification is an ACTION, not an
understanding*, and errors land exactly where checking something would have meant doing something.
`scripts/` and `.github/workflows/` are two different places and only one of them was read.

The note is struck through rather than deleted because the false claim is the useful half. Rung 14's
`profile-legs` matrix is still net new and still worth having; what it does **not** do is close a gap
that was never open.

## Rung 13 — `W9214` has an emitter and a doc page, but no test observes it

`W9214` (telemetry path unwritable) is `Live` in the registry, is raised by `TelemetryStream::open`
and has a `docs/diagnostics/W9214.md` page. Checks 2, 3a and 3b all pass. What does **not** exist is
a test that observes it being emitted, which is the obligation every `Live` row owes.

The reason is that producing it needs an unwritable path, and the ways to get one are all
platform-specific and flaky in CI: a directory that does not exist works on both platforms but is
the least interesting case; a read-only file needs `chmod`/`icacls`; an open-without-sharing needs a
second handle and is Windows-only.

`W9215` and `W9218` **are** observed (`crates/boyko_app/tests/profiling_telemetry_stream.rs`), so
this is one row rather than three. Recorded rather than papered over with a
directory-does-not-exist test that would pass on a typo.

## Rung 14 — `profiling-analysis` is now OPT-IN, and that changes what a plain `cargo build` gives you

**This one is a behaviour change and the owner should know about it before it surprises him.**

`boyko_ecs` used to declare `default = ["profiling-analysis"]`, so every build carried the interval
ring and `ConcurrencyReport` — the "did this schedule actually run in parallel?" answer. It is now
`default = []`, and that answer requires `--features boyko-ecs/profiling-analysis`.

**Why it had to move**, measured rather than argued:

- An environment variable cannot set a cargo feature. Cargo resolves features before any build
  script runs, and `cargo::rustc-cfg` reaches only the crate that emitted it. So `BOYKO_PROFILE`
  could never have switched it, whatever the corpus's table says.
- While it was default-on, **no command line could turn it off**. `cargo tree --workspace -e
  features --no-default-features` still reported it enabled: **nine** sibling manifests depend on
  `boyko-ecs` and not one says `default-features = false`, so unification restored it. Moving the
  request onto a dependency edge would not have helped either — an explicit `features = [...]`
  survives `--no-default-features` by design.
- So the axis's `shipping` row would have been a claim nothing could honour, and `G14(c)` would have
  been a gate with no reachable state.

**What replaces it:** the axis emits `ANALYSIS_ADMITTED`, and `boyko_ecs` refuses at compile time to
be built with the feature on under a profile that does not admit it. The refusal is one-way on
purpose — analysis missing from a `dev` build is a developer who passed fewer flags than they meant
to; analysis present in a `shipping` build is the profile being a lie.

**A coverage consequence, found by asking rather than by a gate.** Six tests in `boyko_ecs`'s
profiling suite are `#[cfg(feature = "profiling-analysis")]`. Opt-in means a bare
`cargo test --workspace --all-targets` no longer compiles or runs them — and a sweep that silently
stops running six tests looks exactly like a sweep that passes. Every CI leg that used to get the
feature from the default list now names it explicitly (`check`, both `test` arms, `clippy`,
`force-alloc-panic`), and the feature-OFF side is covered by the four `profile-legs` builds that
refuse it outright. **A local sweep must pass `--features boyko-ecs/profiling-analysis` too, or those
six do not run.**

**The owner's call, if he wants one:** whether a plain local `cargo build` should carry it. The only
way to have both is to accept that no flag can remove it, which is the state we just left.

## Rung 14 — the symbol census needs `lto = "fat"`, and that raises a separate question about the shipped profile

`G14(a)` is only decidable under LTO. MEASURED on this box, `deep_zone` (one `Deep` zone site,
`boyko_diag` its only dependency):

| link configuration | `dev` | `shipping` | can the gate fail? |
|---|---|---|---|
| default release | `mint_cold` = 1 | 1 | **no** |
| `-C link-arg=-Wl,--gc-sections` | 1 | 1 | **no** — no effect at all |
| `lto = "fat"`, `codegen-units = 1` | 1 | **0** | yes |

The default-release image contains `drop_glue::<boyko_diag::telemetry::Block>` in a binary whose
source never mentions telemetry: the whole rlib rides in and nothing collects it, so a whole-image
census answers "was this codegen'd on the way here?" rather than "can this program reach it?".

The gate passes LTO through `--config` for its own two builds only, so nothing else in the
repository changed. **The open question is a different one:** `[profile.release]` here sets
`codegen-units = 1` for benches and no LTO anywhere. A shipped title almost certainly wants
`lto = "fat"` — it is the difference between `deep_zone` at 1433 symbols and at 4647. That is a
build-configuration decision with compile-time cost, and it belongs to the owner rather than to a
diagnostics rung.

## Rung 14 — `BOYKO_PROFILE=off` does not turn the profiler off, and the thing that would does not exist

`SEAM.md` §S9's table gives the `off` row the tier-column entry *"feature `profiling` off"*.
MEASURED at this rung: **there is no `profiling` cargo feature anywhere in this workspace.**
`boyko_diag` declares `section-gate`; `boyko_ecs` declares `profiling-analysis`, `big_query_table`
and `bench-alloc`; no crate gates `zone!` or `declare_zone!` on a feature at all. And `ZoneTier`'s
three values are `Always`, `Dev` and `Deep` — there is no position below `Always`, so the lowest
compile ceiling the profiler has still admits every `Always` site.

`off` therefore ships as a **logging** off switch: `LOG_CEILING = 0`, `LANE_ARRAY_LEN` becomes
zero-length, which is `G2`'s subject and works. The profiler stays at its floor.

Two ways to close it, both out of this rung's scope: land the FEATURE axis (`G1`) so a `profiling`
feature exists and `#[cfg]` can delete the macro definitions before name resolution; or accept that
"off" means the runtime axis (`ARM_MASK`, `GJ1`) and rename the row. Nothing is blocked either way —
the row is honest as built, and it is written down here because "off" is a word a reader will trust.

## Rung 14 — J1's logging half is owed, and it is owed to rungs that have not landed

`J1` is one rung by construction (S9: one compile axis cannot be split across two rungs), and the
axis is now whole. `L17`'s **other** content is not:

- `LogRuntimePreset` — the five-preset runtime axis.
- The three header facts: `build_profile`, `runtime_preset` and `ceiling` printed as three
  independent values, plus a fixture proving the first two can differ in one binary.
- `G16(d)`, which is the gate over those three fields.
- A **dynamic** logging site in the census fixture. `dyn_debug!` is `L10`'s and does not exist, so
  `G16(a)/(b)` covers the static path only.

All four need a sink header to print into. MEASURED: `boyko_log` has `census`, `codes`,
`drain_owner`, `lane`, `level`, `lifecycle`, `macros`, `rate`, `record`, `site`, `sync_out`,
`target` and `sink/{ecs,file,mod}` — and **no** `sample.rs`, `sink/binary.rs`, `sink/request.rs`,
`sink/crash.rs` or `bin/logdec.rs`. The logging ladder stands at roughly L5 of 17.

This is not a defect in the axis and not a shortcut taken: the axis is the indivisible part and it
landed indivisibly. It is a scheduling fact, recorded so nobody reads "J1 shipped" as "the logging
plan reached L17".

## Rung 16 (J2) — REFUSED ON THE MEASUREMENT: the "both-present" configuration does not exist

J2 is the joint baseline sitting: re-take `zone_cost`, `fold_cost`, `P1` and `P2` **with the profiler
and the logger both present**, in one sitting, and run `GJ1` (the measured off-cost) there. Attempted
after rung 15 and **refused**, because the configuration it is supposed to baseline is not one this
workspace can currently be in. MEASURED:

| | measurement |
|---|---|
| `boyko_log::{error,warn,info,debug,trace}!` across every crate's `src/` | **2 hits, neither an emission site** — a *comment* in `boyko_log/src/lib.rs:86` and rung 15's own `profile_fixture_log` |
| callers of `boyko_log::enable` / `boot` | **none** — no sink thread, no consumer, no panic hook |
| manifests depending on `boyko-log` | **two** (`boyko_ecs`, `profile_fixture_log`); absent from `boyko_app`, `boyko_render`, everything that runs a frame |
| non-test callers of `Profiler::arm` | **none** — rung 11 measured this and it is unchanged |

So `GJ1`'s leg **A** — *"profiler armed, logger enabled, at the shipping ceiling"* — cannot be built,
and legs B and C are defined relative to it.

**Why this is a refusal and not a deferral.** The tempting move is to take the sitting anyway and
stamp the files `both-present`. That is strictly worse than having no baseline: every later
regression gate compares against these files, `config_tag` is what tells a reader the comparison is
legitimate, and a tag that says `both-present` on a run with neither present makes every one of those
gates confidently wrong. The corpus's own rule — *"whichever subsystem landed second must not be
measured against a baseline taken without it"* — is exactly the rule being obeyed here.

**Consequence, stated so it is not read as progress:** the `UNPROVEN` state REMAINS IN FORCE. The
+25 % gate, the revert clauses and `GJ1` still record `UNPROVEN` and still may not fail a rung.

**Preconditions, so the rung can be re-entered rather than re-argued:**

1. Logging **L3** — the sink thread, `enable`, the drain. Without a consumer there is nothing to
   measure the cost of.
2. Logging **L6–L8** — the migration that gives the engine emission sites at all. Today it has none,
   so "logger on" and "logger off" are the same binary doing the same work.
3. ~~A **non-test arm path** for the profiler, so "profiler armed" is a state a shipped frame
   reaches.~~ **DONE, and the note above UNDERSTATED the problem.** It said `Profiler::arm` had no
   non-test caller. Measured immediately afterwards: **`ProfilerPlugin` was added nowhere outside
   tests either** — so the store was not merely unarmed, it was never *inserted*, and fifteen rungs
   of profiler were unreachable from any host. `App::update_with_delta` had been calling
   `fold_frame` all along; it found no `Profiler` and returned. `EnginePlugins` now adds the plugin
   unconditionally — safe by the store's own design, `Profiler::new` *"reserves nothing, commits
   nothing, calibrates nothing"* — and `BOYKO_PROFILE_ON` arms it, which is `SEAM.md`'s route (a).
   Gated by `crates/boyko_app/tests/profiling_host_reachable.rs` (installed + disarmed) and
   `profiling_host_arm_flag.rs` (the flag arms it), both REDs shown.

   **The shape is worth keeping separately from the fix.** Every one of those fifteen rungs was
   green, and none of them could see this: each gate builds its own world and inserts its own store,
   so "does a HOST have one?" was a question no test in the campaign was asking. A subsystem can be
   fully gated and entirely unreachable at the same time.

   **And a second constraint fell out of writing the gate:** `EnginePlugins` **cannot be built twice
   in one process** — the second build panics in
   `register_component_hooks::<boyko_render::light::DirectionalLight>`, because component hooks are
   process-global and the derive's installation is not idempotent. That is why the two legs are two
   test *binaries*. It is pre-existing, it belongs to the render plugins rather than the profiler,
   and it is invisible until something builds the host twice.

**And one defect to fix before the rung, not during it: `config_tag` is already taken.**
`boyko_app::profiling::artifact::config_tag` exists and returns a `String` FNV-1a hash of
`boyko_render::ResolvedRenderPath`'s `Debug` — it identifies the **render path** (Deferred/Forward/VB
× Both/Mesh/Sdf), and `ArtifactHeader::workload_tag` is built from it. S10 asks for a field of the
same name meaning `{profiler, logger}`, in the same crate. Landing it under that name would put two
facts under one identifier, and the failure mode is specific: a reader compares a VB baseline against
a Deferred one, the tag matches, and the difference is reported as a regression. The J2 field needs a
different name (`diag_tag`, say) or the render one does.

---

## L8b — VALUES: L6/L7/L8a silenced 31 diagnostics that used to print unconditionally, and no document says so

**This is not a bug report.** The behaviour is specified and it is gated. It is an owner call about
what a default run of this engine tells its operator, and it is raised here because the migration
rungs took the decision as a side effect of a cost argument rather than as a decision.

**Measured, in this order:**

1. `boyko_app::plugins::boot_and_enable_logging_from_env` calls `boot()` unconditionally and then
   **returns before `enable()`** when `BOYKO_LOG` is unset. `CONTROL` stays `.bss`-zero, so every
   target's runtime ceiling is `Off`.
2. So a migrated `warn!`/`error!` in a default run produces **nothing** — not a dropped record, not
   a counted loss. The macro's third gate is false and the site is one predicted branch.
3. `git show` on the three migration commits: **31 `println!`/`eprintln!` lines were removed from
   production sources** — 3 at L6 (`49cf2230`), 12 at L7b (`b30fa810`), 16 at L8a (`1a76e4a9`).
   Spot-checked against the parent commit, `boyko_render`'s `W2201` site was an **unconditional**
   `eprintln!` behind a one-shot latch, not an env-gated or `debug_assertions`-gated one.
4. The silence is **deliberate and pinned**: `logging/sink-lifecycle` Decision 25 states *"a
   flag-off run of any other preset configures nothing either, because `enable()` never ran and no
   sink slot was ever opened"*, and `crates/boyko_app/tests/log_host_reachable.rs` asserts
   `flush() == NoConsumer` after a full `EnginePlugins` build, calling it *"the half that makes the
   cost claim true rather than merely stated"*.

**What no document in the corpus says** is what (4) does to (3). The plan gated the *cost* — one
sink thread, a 20 ms clock calibration in `enable()`, a panic hook — and in doing so gated the
*diagnostics*, and the migration rungs then converted 31 unconditional prints into records behind
that gate without the trade being written down anywhere.

**The question, stated as a fork:**

* **(A) Diagnostics stay opt-in** (today's behaviour). A shipped run is silent until an operator
  sets `BOYKO_LOG`. Cost: nothing. Consequence: a `Warn` nobody sees is a `Warn` that does not
  exist, and the engine's 24 Live `W`/`E` codes are documentation rather than diagnostics.
* **(B) The host enables at a `Warn` floor unconditionally**, and `BOYKO_LOG` raises it. Cost:
  ~20 ms of clock calibration and one sleeping thread per process — including every child process
  the test suite spawns. Consequence: the pre-migration behaviour is restored and the codes become
  reachable without foreknowledge.
* **(C) Wire the synchronous route.** `TargetControl::SYNC_BIT` is declared, `sync_out` exists, and
  `emit_unlaned_line` already renders and writes through it — but the bit **has no reader**:
  `lane.rs`'s only synchronous path is the no-lane fallback, not a route the control byte can
  select. Wiring it would let `error!` reach `stderr` with no thread and no calibration. This is
  the architecturally right answer and it is L12-shaped work, not L8b's.

**L8b did not wait on this.** The three terminal-exit codes (`E3002`/`E3003`/`E3004`) fall back to
`eprintln!` when `flush()` answers `NoConsumer`, on `boyko_threadpool::worker`'s already-blessed
precedent — so the host cannot exit silently under any of the three answers. The degrade codes and
the thirty `info!` sites follow (A) as specified. **The 31 already-silenced sites from L6/L7b/L8a
are untouched and remain silent**, which is what this entry is about.

---

## L8b — `boyko_app` never calls `flush()` or `shutdown()`, so SEAM S5's teardown half does not exist

Measured: `boyko_app` names `boyko_log::lifecycle` in exactly one place, `plugins.rs`, and calls
`boot` and `enable`. There is no `flush()` and no `shutdown()` anywhere in the crate.
`SEAM.md`'s S5 gives `boyko_app` *"the boot and teardown order, `flush_gpu` ahead of `flush`"*; the
boot half landed at L7 and the teardown half never did.

The consequence is not theoretical. `lifecycle::enable` spawns the sink thread and **drops its
`JoinHandle`** (`.spawn(sink_loop).map_or_else(…, drop)`), so nothing joins it. A record emitted
just before `return AppExit(true)` races the drain and loses more often than not — and the sites
L8b migrates are exactly the print-then-exit ones.

L8b's three terminal reporters call `flush()` themselves, so those records leave. **Every other
record emitted late in a run is still exposed**, including the `VB-ZONE summary` and artifact lines
that a measurement run ends with. The fix is a `flush()` on the normal teardown path and a
`shutdown()` after it, and it wants doing with the rung that owns the lifecycle rather than bolted
onto this one.

---

## L8b — deleting `boyko_demo`'s `log` facade left two channels with no replacement

The ledger specifies the deletion of `log = "0.4"`, `env_logger` and `console_log` from
`boyko_demo`, and L8b did it. Two things went with them:

* **Native**: `env_logger` was the only subscriber for the `log` facade in that binary, and
  `eframe`/`egui`/`wgpu`/`naga` all emit through it. **wgpu adapter selection and validation
  messages now go nowhere.** The replacement is a `log`-facade bridge feeding `boyko_log`, which no
  rung owns.
* **wasm**: `console_log` was the only channel reaching the browser console, and `boyko_log`'s
  console sink writes to `stderr`, which is a no-op on `wasm32-unknown-unknown`. So `E3001` — the
  record whose entire purpose is to explain a blank canvas — is emitted and unreachable there.
  It costs nothing **today**, because the wasm build is blocked upstream in `boyko_ecs` (the layout
  asserts fail 32-bit const-eval) and its CI leg is explicitly non-fatal. Whoever unblocks wasm
  owes the console sink a `web_sys::console` arm, or the failure goes back to being silent.

Both are recorded in `crates/boyko_demo/Cargo.toml` beside the dependency, so the next reader of
that manifest finds them without finding this file.

---

## L8c — four `Pending` code rows name profiling rungs that have SHIPPED, and all four conditions exist and are silent

Check 3c (`Pending == 0`) is L8c's, and it cannot arm while these four rows stand. Measured against
HEAD, each condition **exists in the tree and reports nothing**:

| row | condition, located | state |
|---|---|---|
| `W9202` `Pending("profiling 5")` | `boyko_rhi_vulkan::present::gpu_zone::alloc_pair` returns `None` once `used_pairs >= MAX_GPU_PAIRS` (128) | the bracket is simply unrecorded; nothing reports it |
| `W9217` `Pending("profiling 5")` | `runner.rs` calls `flush_vb_zone` **only** inside `vb_zone_seen >= WARMUP + frames` | a run that ends earlier — window closed, or `E3003`'s terminal `return` — leaves slots in flight, unflushed, unreported |
| `W9205` `Pending("profiling 8")` | `reduce.rs` increments `census.lost`; `contrast.rs` reads it as `window_complete` | counted, carried into the artifact, never warned about |
| `W9206` `Pending("profiling 8")` | `contrast.rs` has `NotResolved` + `NotResolvedReason` fully built | a refusal is returned; nothing warns |

**`GpuZoneRecorder::flush` itself is correct** and labels every in-flight slot `Flushed` — its own
doc says it exists so *"the last `GPU_RING_DEPTH` slots would [not] be dropped silently, which is
the loss a profiler exists to report rather than to commit"*. `W9217`'s hole is not in `flush`; it
is in the one path that never calls it.

**Also measured, and it is the shape of the thing:** `boyko_app::profiling` contains **zero**
`warn!`/`error!` calls across fifteen shipped rungs. The `92xx` emitters all live in
`boyko_ecs::…::profiling::diag`, which its own header names as the **sole** emitter of the block —
*"which is what keeps a profiler drop reported as a counter read rather than as a log record that
can itself be dropped under exactly the load that produced the drop"*. So these four do not become
`warn!` at the condition site: they route through `boyko_diag::loss::raise(DiagFlag::…)` sticky
bits that `diag.rs` reads. `flag_code`'s `match` is deliberately not `_`-terminated, so a new
`DiagFlag` variant **fails to compile** until it is paired with a code — the mechanism is already
built and simply has four unused inputs.

**The question for the owner is not how, it is whether these belong to L8c at all.** They are
profiling conditions, in profiling crates, whose rungs are marked shipped. L8c inherits them only
because `Pending == 0` is its gate. Either the profiling ladder reopens rungs 5 and 8 to land the
emitters it reserved codes for, or the four rows are re-dispositioned. Recorded rather than decided
because it moves work between two ladders.

---

## ~~L13b's revert clause has fired~~ — RESOLVED 2026-08-17: keep L13b, the 5× was an estimate

**OWNER RULING: keep L13b. The `5×` was an estimate, not a requirement.** Recorded below as it was
asked, because the measurement is the reason the threshold moved and a resolved question that
deletes its own evidence teaches nothing.

**What changed in the tree**: `02-SINK-LIFECYCLE.md`'s clause is re-cut from an acceptance
threshold into a **regression guard** at `≥ 4.0×` and `≥ 3 M rec·s⁻¹`, set from the four readings
and deliberately below the observed minimum rather than pinned to it — a bound at today's number
reds on ordinary variance, and a gate that cries wolf gets ignored. The bench prints `PASS` /
`REGRESSION` instead of `PASS` / `FAIL (revert clause)`, and its RED was shown by raising the guard
to `6×`.

**The lesson the corpus keeps**: the `5×` was written before anything was measured and nothing was
ever measured against it until the bench existed. A number invented in advance is a guess about the
answer; this corpus does not get to hold a guess and a measurement in one sentence and call the
guess the requirement.

`02-SINK-LIFECYCLE.md` states the clause without hedging: *"the entire justification is throughput.
If `sink_sustained_rate_binary` does not measure ≥ 5× `sink_sustained_rate` in the same sitting,
**L13b is reverted**. A format whose only reason to exist is speed must show the speed."*

The bench now exists (`crates/boyko_log/benches/sink_sustained_rate.rs`) and it was built to answer
exactly this. Four sittings on this box:

| sitting | text ns/rec | binary ns/rec | ratio | A-vs-A' twin gap |
|---|---|---|---|---|
| 1 | 41.02 | 9.54 | **4.30×** | 0.020 ns |
| 2 | — | — | **4.63×** | 0.085 ns |
| 3 | — | — | **4.68×** | 0.120 ns |
| 4 | — | — | **4.54×** | 0.080 ns |

**The instrument is sound and the result is not marginal noise.** The A-vs-A' twin — the same leg
measured twice around the other — drifts by 0.02–0.12 ns while the legs differ by ~31 ns, so the
sitting is not drifting. The separation is ~31 ns against a combined spread floor of ~1 ns, so it
resolves. Four independent sittings land in a 0.38× band, none of them touching 5×.

**The absolute half of the clause passes by a wide margin**: 104.8 M rec·s⁻¹ against a floor of
3 M. It is only the *ratio* that misses.

**And the measured scope is the one most favourable to L13b.** The bench times only where the two
paths differ — `render_payload` against `encode_record` — because everything upstream of the drain
and downstream of the sink's `write` is shared. An end-to-end sink rate would add a constant both
formats pay, which can only push the ratio *down*. So 4.5× is an upper bound on the end-to-end
figure, and the clause still misses.

**The three dispositions, and why this is not mine to pick:**

1. **Revert L13b as written.** The clause is unambiguous and the number is reproducible. Costs:
   `binary.rs`, its dictionary, `W0116`, the format tests and the offline decoder plan all go.
2. **Keep it and amend the threshold.** 4.5× at 105 M rec·s⁻¹ is a real improvement; a 5× line
   drawn before anything was measured is not obviously the right line. This requires the owner to
   say the threshold was the estimate, not the requirement.
3. **Keep it and make it faster.** The text leg's 41 ns is dominated by `core::fmt`; the binary
   leg's 9.5 ns is already close to a `memcpy` of 39 bytes. The ratio is more likely to move by
   *slowing nothing and speeding the text leg less* than by optimising the binary one — i.e. this
   route probably does not reach 5× without changing what the text sink does.

Each of these trades shipped, tested, documented code against a number, which is a values call, not
a performance fork. Recorded and surfaced rather than decided.

### Correction to the readings above, made after the instrument was fixed

The four sittings quoted in the table were taken with a spread floor that was **2 % of the reading
and nothing else** — the IQR was exactly zero, so `se.max(med * 0.02)` reported the subject's size,
not the clock's resolution. Fixing that (`benches/instrument.rs`) and re-measuring gives a fuller
picture:

* **Eleven sittings on an idle box: 4.29× – 4.68×.** The original four sit inside that band, so the
  ruling rests on the same evidence it always did, and the `4.0×` guard keeps its margin.
* **One sitting read 5.94×**, taken immediately after a build with the machine still busy. It is
  not a contradiction: the TEXT leg is the load-sensitive one, so load inflates the ratio.
* **The A-vs-A' twin test was too lax and has been re-cut.** It compared the twin gap against the
  ~32 ns *separation*, which admitted a sitting that drifted 3.4 ns on a 41 ns leg — 8 %, enough to
  move the reported ratio by ~0.35, wider than the entire band. It is now proportional: the twin
  must agree to within 2 % of the leg.
* **That change caught a false red.** A drifted sitting produced `2.61×`, which under the old test
  would have been reported as `REGRESSION` — an accusation against a format that had lost nothing.
  It is now correctly `NOT MEASURABLE (instrument)`.

The direction of the ruling is unaffected. What changed is that "reproducible" is now a claim about
an idle box with a drift-rejecting twin, rather than a claim resting on byte-identical numbers that
were byte-identical because the floor was fictional.

---

## `Once` is now the ONLY policy honoured by hand, and 39 sites do not obviously honour it

**Status:** OPEN — measured 2026-08-19, not fixed. Raised because the fix is a rung, not a footnote.

`rate::admit` is wired (`__log_rate_admits!`, the fourth gate). `Every`, `EveryN` and
`MinIntervalMs` are now applied mechanically by the emission macros. `Once` and `OnceCounted` are
NOT, deliberately: the latch stays a named `OnceSite` the site declares, because a `static` inside
a macro expansion cannot be named and `OnceSite::reset` exists precisely so an observer can reset
the latch it is about to test.

That leaves `Once` as the last policy whose declaration is kept by human diligence — which is this
campaign's signature defect shape. **Measured across `crates/**/*.rs` and `src/**/*.rs`, excluding
`codes.rs`, `tests/` and `benches/`: 45 `Live` rows declare `Once`/`OnceCounted`, and 39
(identifier-use, file) pairs have NO `OnceSite` anywhere in the file.** Two were read by hand and
are real:

* **`W0111` (`crates/boyko_log/src/census.rs:122`, `report_unsunk`)** — `#[cold]`, no latch, called
  from inside `census::rows()`, which is a **public iterator** any host may walk per frame. Its own
  doc comment says `Once`, "because the condition is a CONFIGURATION and not an event". A per-frame
  census overlay would emit it once per unsunk target per frame.
* **`E0109` (`crates/boyko_log/src/sink/crash.rs:82`, `report_unopenable`)** — `#[cold]`, no latch.
  It fires once today only because `arm()` is called once on the enable path; the row's `Once` is
  honoured by the CALL STRUCTURE, not by anything at the site.

The crude scan cannot tell an emitter from a mention (a `use`, a doc link, a test assertion), so 39
is an upper bound and the real count needs an emitter-aware walk — the shape `code_registry.rs`'s
existing checks already have.

**Two dispositions, and the second is the one that needs a ruling:**

1. **Audit the 39 and place the missing latches.** Mechanical, site by site, and each site's
   correct granularity is a judgement (`W2205` deliberately keeps its latches in a `Resource`, not
   a `static`, so one world's first divergence cannot silence another's).
2. **Place the latch in the macro after all, and make it resettable.** The objection above is
   testability, and it is answerable: register every macro-placed latch against its `&LogSite` in a
   walkable table and give `test-probe` a `reset_all_once_sites()`. Then all 45 rows are honoured
   mechanically and an observer resets everything before driving its site. This changes behaviour
   at 45 rows and costs `.bss` plus a registration on first emission, so it is a scope call.

Recorded rather than decided.

**Addendum (same session): the corpus's own accounting for `Once` is not built either.**
`00-GOAL-TARGETS.md:37`, `01-EMISSION-RING.md:273` and `05-LADDER-GATES.md:918` all specify an
`ONCE_SITES` intrusive list and one `LOG-ONCE` census row per fired site
(`code=W2102 site=device.rs:3100 fired=1 suppressed=UNCOUNTED(by policy)`). **Neither exists.** Two
doc comments in the tree named it as though it did — `crates/boyko_log/src/rate.rs` called it "the
`ONCE_SITES` walk's answer", and `crates/boyko_rhi_vulkan/src/present/passes/gbuffer.rs` claimed a
site "enrols itself in `ONCE_SITES` so the `LOG-ONCE` census can report that it fired at all". Both
corrected in place with the wiring commit.

This is the same finding as the 39 latch-less sites, from the other end: **nothing enumerates
`Once` sites, so nothing could notice.** Building `ONCE_SITES` + the `LOG-ONCE` rows is the natural
pair to disposition 1 above — the audit needs the enumeration, and the enumeration makes the audit
mechanical instead of a grep.

---

## UPDATE (same day): the enumeration is BUILT, two of the sites are fixed, and the audit is now a number

`crates/boyko_log/src/once_sites.rs` is the register the corpus specified. The drain notes every
emission from a site whose `LogSite::rate` is `Once`/`OnceCounted` — off the emitting thread, from
cold `'static` data — and `census::print` emits one row per fired site:

```
LOG-ONCE code=W2102 site=crates/boyko_rhi_vulkan/src/device.rs:3100 fired=1 suppressed=UNCOUNTED(by policy)
```

**`fired > 1` is the defect, stated as a number.** The row even says so:
`  <-- DECLARES Once AND HAS NO LATCH`. The 39-pair grep was an upper bound with no way to tighten
it — it cannot tell an emitter from a `use` or a doc link — and this replaces it with a per-site
run-time count.

**Two engine sites are fixed with it, and the first was found BY it:**

* **`W0111`** — `report_unsunk` was called from inside `census::rows()`, a **public iterator** a
  host may walk every frame. Reverting the fix and walking `rows()` ten times makes the register
  read `fired: 10` for `census.rs`. The report moved to `census::print()` (flush and shutdown)
  behind a named `UNSUNK_REPORTED` latch. **A query must not have a diagnostic as a side effect**,
  which is the general form of this defect.
* **`E0109`** — `report_unopenable` had no latch; its `Once` was honoured by the call structure
  (`arm` runs once on the enable path), so a process that disabled and re-enabled would report
  again. It has a named `UNOPENABLE_REPORTED` latch now.

**What is still open.** The remaining ~37 pairs are not audited: the register only reports sites
that FIRE, and a site that never fires in a test run leaves no row. Reading it properly means
running the engine and reading the census, which is the next step rather than a grep. The
macro-auto-latch question (disposition 2 above) is untouched and still needs a scope call.

---

## The certification recipe never runs 311 tests, and 177 of them do not say why

**Status:** OPEN — measured 2026-08-19. Surfaced rather than half-fixed, because the fix is a
policy call.

The two-half recipe in `CLAUDE.md` reports green without running a single `#[ignore]`d test:

```
cargo test --workspace --exclude boyko_rhi_vulkan --all-targets --no-fail-fast
cargo test -p boyko_rhi_vulkan --all-targets --no-fail-fast -- --test-threads=1
```

Measured across `crates/`, `src/` and `tests/`: **311 `#[ignore]` sites in 90 files.**

**Do not read that as 311 defects.** Of the 134 that state a reason, 122 name a GPU, device,
window or swapchain requirement, 3 name process-wide state and 2 name a dump or golden — all
legitimate reasons for a test to be driven by hand. The number worth acting on is the other one:

**177 sites carry `#[ignore]` with NO reason at all.** They sit in 78 files, mostly windowed/GPU
suites whose module docs do explain the requirement — so 177 is an upper bound on "silenced with
no record", not a defect count. But a bare `#[ignore]` cannot be told apart from a test that went
red once and was quieted, and that is precisely the distinction this campaign exists to make
mechanical.

**One instance is verified and load-bearing right now.**
`crates/boyko_log/tests/l14_sink_policy.rs`'s `an_armed_target_no_sink_accepts_is_unsunk_and_says_so`
is the **only** observer of the `W0111` latch this rung introduced, and the recipe does not run it.
It was run by hand for this commit (`-- --ignored --test-threads=1`, green) and that is a process
step, not a gate.

I did not merge it into its sibling: the second test re-`boot`s and re-`enable`s, and `enable()`
on an already-enabled process is a different code path — a merge could produce a test that passes
for a new and wrong reason, which is worse than one that does not run.

**Two questions for the owner:**

1. Should the recipe gain a third leg, `-- --ignored --test-threads=1` per crate — accepting that
   the GPU/windowed suites will then need a device present?
2. Should a bare `#[ignore]` become a tidy-check failure, so that switching a gate off always
   leaves a written reason? That is the same rule as the mandatory `// SAFETY:` comment and the
   `#[allow(clippy::disallowed_types)]` rationale, applied to the third way to make a check
   disappear.

---

## RESOLVED: the 39 was an upper bound, and the sharpened count is 19 latched of 20

**Status:** CLOSED 2026-08-19 — the residue is named below and is small.

The entry above reported "39 (identifier, file) pairs carry no `OnceSite`" and said in the same
breath that it was an upper bound with no way to tighten it, because a grep for identifier USES
cannot tell an emitter from a `use`, a doc link or a test assertion. It has now been tightened, and
the answer is different in kind:

**Emission-aware, production-code-only: 20 files emit a `Once`/`OnceCounted` code. 19 hold a latch.
The one flagged was `boyko_log/src/macros.rs`, whose `warn!` DOC COMMENT explains the class/number
pairing using `W2102` as its example** — not an emission at all, and gone once the scan reads the
production stream instead of raw text.

**A middle draft was wrong in the other direction and is worth recording.** Requiring the latch
inside the emitting FUNCTION returned 21 — of which **19 were correct code**:
`boyko_ecs::…::profiling::diag` holds one `OnceSite` per live code in a `LATCHES` array behind a
`claim(number)` helper, so no emitter there names `.claim()` itself. A gate written that way would
have accused nineteen sites that latch properly. Measuring before writing the check is what caught
it.

**Two real defects came out of this and are fixed**, both found by the run-time register rather than
by either grep: `W0111` emitting from inside `census::rows()`, a public iterator a host may walk
every frame; and `E0109`, whose `Once` was honoured by the call structure rather than by anything at
the site.

**The gate is `check_8_every_file_emitting_a_once_code_has_a_latch`** in
`crates/boyko_log/tests/code_registry.rs`, with an anti-vacuity floor (a scan finding fewer than ten
emitting files fails rather than passes) and two REDs shown.

**What it still cannot prove**, stated in the check's own doc: that the latch guards THAT emission
rather than another in the same file. That residue is `boyko_log::once_sites`' at run time, where a
`Once` row reading `fired > 1` is the defect stated as a number.

**Disposition 2 above — placing the latch in the macro — is therefore NOT needed** and is withdrawn
as a question. The human link is now checked mechanically at build time and observably at run time,
which is what auto-latching was going to buy, without making all 45 rows untestable in isolation.

---

## 2026-08-20: the census reaches only the console — a `shipping` log never carries its own loss summary

**Context.** The windowed runner now ends the diagnostics session (`lifecycle::shutdown()` at the
end of `run_windowed`), so `close_out` runs in every real host: the final drain delivers the lane
tail and `census::print()` fires. Measured on live 16-frame runs of the `clear` example.

**The question.** `census::print` writes through `sync_out::write_oracle_line` — the synchronous
console channel — and through nothing else. Under `dev`/`editor` (console on) the census reaches
the operator. Under `shipping` (binary file, console off) and `shipping-min` (text file, console
off) the rows are refused at the console gate and reach **no destination at all**: the uploaded
log a released title produces does not say whether it lost anything, which is the one question a
reader of that log asks first.

**Two dispositions, neither taken without you:**

1. **Route the census through the ring** (ordinary records, `Diag` target) so it lands in
   whatever sinks the preset opened. Cost: the census becomes subject to the same admission
   control it reports on — a storm that drops records could drop the census rows that say so.
   The current synchronous channel exists precisely to be outside that machinery.
2. **Render the census into the file sinks directly under the drain token at `close_out`**,
   beside the console write. Cost: a second delivery path for one report, and the binary sink
   would carry text lines outside the record format (or needs a frame kind for them).

**RESOLVED same day, by the owner's standing directive to decide without asking (2026-08-20 chat).
Disposition 1 was taken, narrowed to shutdown-time.** Disposition 2 fell to a fact discovered on
inspection: the `.blog` is a framed format, so "write the rows into the sink directly" means
constructing record frames anyway — which IS disposition 1 with extra steps. `census::print` now
emits every row as an ordinary ring record (`LOG-CENSUS …` / `LOG-ONCE …` under the `log` target),
`shutdown` orders itself "emit → deliver → close" in both arms, and the sinks — the binary one
included, which previously had no shutdown-time close at all — close only after the final pass.
The stated cost stands and is accepted: the census obeys the admission control it reports on,
which at shutdown means a quiet ring and an immediate delivery pass. Gate:
`log_host_shipping_min.rs` asserts `LOG-CENSUS` rows in the preset's own file after `shutdown`.
