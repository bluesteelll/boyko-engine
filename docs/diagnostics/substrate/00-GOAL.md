# Substrate — goal, crate graph, and the mute-leaf rule

<!-- CONTRACT
provides: substrate/dedup-rationale     # why boyko_diag exists; the four duplications it removes
provides: substrate/crate-graph         # the 21-manifest edge list and the acyclicity proof
provides: substrate/mute-leaf-rule      # the leaf emits nothing; report-above-the-leaf and its costs
-->

**This file declares no `assumes:`, and that is a decision, not an omission.** The substrate is
the bottom of the corpus: it may assume nothing outside itself, and inside itself this file is
the root — every other substrate file reaches it, four directly (`02-LANE` assumes
`substrate/crate-graph`, `03-LOSS` assumes `substrate/mute-leaf-rule`, `04-STORAGE` assumes
`substrate/dedup-rationale`, `05-LADDER-GATES` assumes both) and `01-CLOCK` transitively through
`03-LOSS` and `04-STORAGE`. Its links back to those five are navigation between parts of one
argument, not dependencies, which is why declaring **any** of them would close a cycle rather
than describe one. **No substrate file names, cites or derives anything from a `seam/`,
`profiling/` or `logging/` file.** A bottom layer that justifies itself by naming its consumer
has inverted the layering, and the inversion survives review precisely because it reads as a
helpful cross-reference.

> **Carved from** `docs/DIAGNOSTICS-SUBSTRATE-PLAN.md` — the preamble, §1 **minus the joint cost
> table** (an inherently *joint* number, so it is not carved into this area at all), §2, §3's
> crate-attribute preamble, §4, §5 and §6.
> Diff against that file until the monoliths are retired.

**Status:** design, pre-implementation.

**Provenance.** This document implements the architect's seam decision record answering the
round-3 seam review of `docs/PROFILING-SYSTEM-PLAN.md` × `docs/LOGGING-SYSTEM-PLAN.md`
(verdict: INCOMPATIBLE AS WRITTEN, findings S1–S12). The decision record is the approved
design; this corpus is its implementable form. It does not re-open any decision. Where a
statement in the record did not survive verification against the tree, the divergence is
recorded in [`05-LADDER-GATES.md`](05-LADDER-GATES.md) and **the tree wins** — a plan that
contradicts the manifests is a plan that reds on its own first rung.

**Scope.** ONE new crate, `crates/boyko_diag`, and ONE new edge into `boyko_threadpool`. Two
rungs: **D0** (the crate) and **D1** (the lane write sites). Everything downstream of D1 —
`boyko_log`, `profiling_abi`'s move, the **18** `W92xx` rows (`W9201`..`W9218` — consecutive, no
gaps, so the count is the range), the five build profiles — belongs to the two subsystem plans
and is referenced here, never restated.

**This crate's whole value is that it is small.** A shared bottom crate that accretes is the
same Principle-0 defect as two subsystems each minting their own copy, pointed the other way.
The growth rule below is load-bearing and is quoted verbatim from the decision record.

---

## 1. Goal, and why this crate exists at all

`boyko_diag` **removes** four duplications before they are written. It adds no capability that
either subsystem could not have built for itself; it makes it impossible for them to build it
twice and disagree.

| Primitive | If each subsystem keeps its own copy |
|---|---|
| **A1 clock** | A suspend/resume produces a profiler window quarantined as `EpochBreak` and, in the same seconds, log lines whose printed wall times are wrong by the suspend duration with no marker — two artifacts that disagree, neither of which says why. |
| **A2 lane** | The same worker is lane 5 to the profiler and lane 37 to the logger, so no reader can place a log line inside the zone it happened in — the one joint question the pair exists to answer is unanswerable by construction. |
| **A3 loss** | The profiler reports its own drops *through* the logger, so under load — precisely when profiler drops occur — the report of the loss is dropped and counted as a *logger* loss; two counters double-count one event and no rule says which is authoritative. |
| **A4 storage** | Two residency proofs over two statics with two demand-zero arguments, and the toolchain behaviour both admit is unproven (PE/COFF `.bss` placement) gets proved twice — so a toolchain change reds one gate and not the other, and the reader cannot tell which is authoritative. |

Each primitive has its own file: [`01-CLOCK.md`](01-CLOCK.md), [`02-LANE.md`](02-LANE.md),
[`03-LOSS.md`](03-LOSS.md), [`04-STORAGE.md`](04-STORAGE.md).

### The honest quantitative statement

**No joint cost table is stated here, and no CURRENT figure from one is cited here** — the single
exception is the withdrawal note below, which quotes a *superseded* row in order to show why a
claim was retired. Quoting a dead number to bury it is not restating a live one; quoting a live
one here would be, and is forbidden. The table is inherently a *joint* number, and three separate
statements of one figure is precisely how this corpus came to contradict itself. What this file
owns is the standing rule — and the rule needs no figure at all:

> **The substrate is bought for correctness — one lane number, one epoch, a loss report that
> cannot itself be dropped — never for footprint. No rung of any plan may be justified by a byte
> count**, and no plan may offer a footprint saving as this crate's reason to exist.

**The "saves 0.78 MiB in dev" claim is withdrawn, not restated elsewhere.** It was arithmetic
over a superseded revision of the substrate plan's own table
(`docs/DIAGNOSTICS-SUBSTRATE-PLAN.md:43`, dev row `6.65 | 3.46 | 10.11 | 9.33`: the naive sum is
`6.65 + 3.46 = 10.11`, and the saving is `10.11 - 9.33 = 0.78`). Both operands were re-cut
afterwards, so the naive sum they produced no longer exists and the difference derived from it
cannot be recomputed. **The dev saving is therefore UNQUANTIFIED at this revision**, and an
unrecomputable number is exactly what the rule above forbids a plan to lean on. The shipping
figure was always **zero saving** and that half stands.

*(The withdrawn sentence carried one distinction worth keeping on its own: any such figure would
be about **reserved extents** — how much address space the declared tables span — never about
resident bytes. Reserved extent and residency are different claims, and the limit of what this
corpus can prove about the second is stated once, in [`04-STORAGE.md`](04-STORAGE.md).)*

Two qualifications this document adds, because a joint table is otherwise read as stronger than
it is:

- A "1 TLS slot" row counts *diagnostics* slots. `boyko_threadpool::tls::CURRENT_WORKER_ID`
  is untouched and remains, so a worker thread holds **two** TLS `Cell`s after D1: the pool's
  own worker id and `boyko_diag::LANE`. The second exists only because `boyko_diag` sits
  *below* `boyko_threadpool` and therefore cannot call `current_worker_id()`
  ([`02-LANE.md`](02-LANE.md)).
- `boyko_diag`'s own `.bss` (≈ 42 KiB, the same in every profile — nothing here is sized by
  `BOYKO_PROFILE`) must be attributed **exactly once**. Those bytes are
  already inside the two subsystems' own rows, because the move is where they came from;
  counting them again as a new row manufactures a footprint regression out of a relocation.

---

## 2. Crate graph

`->` = a `[dependencies]` edge. **Every edge below was re-read from the real `Cargo.toml`**;
deltas against the decision record are in [`05-LADDER-GATES.md`](05-LADDER-GATES.md).

```
NEW BOTTOM
  boyko_diag        -> {}                          std only; zero workspace, zero third-party
  boyko_log         -> boyko_diag, boyko_macros    (logging plan; not this document)

UNCHANGED LEAVES (no new edge)
  boyko_utils       -> {}                          <- STAYS ZERO-DEP (Cargo.toml:6, empty table)
  boyko_math        -> {}
  boyko_shaderdsl   -> {}
  boyko_macros      -> syn, quote, proc-macro2
  boyko_sdf_math    -> boyko_shaderdsl             no_std; cannot and does not take boyko_diag
  boyko_image       -> {}                          + boyko_log at L8b (logging plan)
  boyko_fontbake    -> boyko_math, boyko_threadpool, ttf-parser

EXISTING + NEW EDGES (new marked +)
  boyko_threadpool  -> crossbeam-deque, crossbeam-utils, [cfg(loom)] loom,
                       +boyko_diag                 <- THE ONLY EDGE THIS PLAN ADDS (D1)
  boyko_rhi         -> boyko_utils
  boyko_rhi_vulkan  -> boyko_rhi, boyko_sdf_math, [cfg(windows)] windows-sys
  boyko_ecs         -> boyko_utils, boyko_threadpool, fixedbitset, crossbeam-queue,
                       crossbeam-utils, static_assertions, [cfg(unix)] libc, [cfg(loom)] loom
  boyko_input       -> boyko_ecs, boyko_utils, boyko_macros, [cfg(windows)] windows-sys
  boyko_scene       -> boyko_ecs, boyko_math, boyko_macros, boyko_input
  boyko_physics     -> boyko_ecs, boyko_macros, boyko_utils, boyko_threadpool,
                       boyko_sdf_math, boyko_math, boyko_scene
  boyko_serialize   -> boyko_ecs
  boyko_render      -> boyko_ecs, boyko_macros, boyko_rhi, boyko_rhi_vulkan, boyko_sdf_math,
                       boyko_scene, boyko_math, boyko_fontbake, boyko_image, bytemuck
  boyko_ui          -> boyko_ecs, boyko_macros, boyko_input, boyko_fontbake, boyko_scene,
                       boyko_math
  boyko_app         -> boyko_ecs, boyko_scene, boyko_render, boyko_rhi, boyko_rhi_vulkan,
                       boyko_input, boyko_math, boyko_sdf_math, boyko_macros
  boyko_demo        -> boyko_ecs, boyko_macros, boyko_threadpool, eframe, bytemuck,
                       log 0.4, rand, [not wasm] env_logger, [wasm] console_log + 4 more
  bench_bevy_vs_boyko -> mimalloc(opt); dev-deps: boyko_ecs, boyko_macros, boyko_threadpool,
                       bevy_ecs, criterion
  boyko-engine (root package, `.`) -> {}
```

**Twenty-one manifests, not nineteen.** `members` lists twenty crates and `default-members`
lists the same twenty **plus `"."`** (root `Cargo.toml:2` and `:13`), so any workspace-wide tidy
check must enumerate **21** manifests. `bench_bevy_vs_boyko` and the root package `boyko-engine`
are both `members` and `default-members` and are easy to omit; the list above is complete.

**Acyclicity proof.** `boyko_diag` has out-degree 0 — it names no workspace crate and no
third-party crate, so no path leaves it and no cycle can pass through it. The single new edge
`boyko_threadpool -> boyko_diag` therefore cannot close a cycle: a cycle through that edge
would require a path `boyko_diag ->* boyko_threadpool`, and `boyko_diag` has no out-edges at
all. The same argument covers every later `X -> boyko_diag` edge the two subsystem plans add.
`boyko_log`'s only out-edges are `boyko_diag` (out-degree 0) and `boyko_macros` (out-edges
`syn`/`quote`/`proc-macro2`, all external), so no workspace crate is reachable from
`boyko_log` either.

**The profiling plan's two planned edges `boyko_rhi_vulkan -> boyko_utils` and
`boyko_threadpool -> boyko_utils` are withdrawn** and replaced by `-> boyko_diag`. At the D0/D1
scope only the `boyko_threadpool` one lands; `boyko_rhi_vulkan`'s arrives with profiling
rung P1.

**Two file-level consequences.**

1. `crates/boyko_image/Cargo.toml:5` — the package description reads "…Decoupled leaf utility
   crate — **no dependency on any other workspace crate**, mirrors boyko_utils' role for image
   data." Verified present, verbatim, and verified false the moment L8b adds `boyko_log`. The
   description is edited in the same commit as the edge, by the logging plan, not here.
2. `crates/boyko_rhi_vulkan/Cargo.toml` — its INVIOLABLE no-third-party rationale is at
   **`:7-12`** (**not** `:44-49`, which is the `boyko_sdf_math` *precedent* block). The
   `boyko_diag` row is added at `:7-12` citing the `:44-49` precedent: an in-house, zero-dep
   sibling workspace leaf does not breach "no ash / vulkano / windows-sys / libc".
   *The new rationale row goes **after** `:12` and before the `[features]` header at `:13`;
   an insertion **at** `:12` would land between `:11` and `:12` and split the sentence that
   spans `:10-12`. Block extent and insertion index are two different numbers.*

**Not an edge, and must not become one:** `boyko_utils` does not gain `boyko_log`. The logging
plan's Decision 15 sentence "`boyko_utils` depends on `boyko_log`, not the reverse" is struck.
Nothing in `boyko_utils` logs; its four modules are `bit_mask`, `identifiers`, `sparse_map`,
`type_intern`.

---

## 3. Crate-level attributes

Two layers. **Layer A** is shared — both subsystems write it, and it is the subject of
[`01-CLOCK.md`](01-CLOCK.md) … [`04-STORAGE.md`](04-STORAGE.md). **Layer B** is `profiling_abi`,
which is profiling-only and lives here for the graph reason in §5, not because it is shared.

Crate-level attributes on `crates/boyko_diag/src/lib.rs`:

```rust
#![deny(clippy::print_stdout, clippy::print_stderr, clippy::dbg_macro)]
#![deny(missing_docs)]
```

The three `print` lints are the mute-leaf rule's mechanical half (§6): they ride the existing
`cargo clippy --all-targets -- -D warnings` gate, so **no new CI job is needed**.

---

## 4. Explicitly NOT owned

| Not owned | Why |
|---|---|
| Any allocator, `VmReservation` | `VmReservation` is `pub(crate)` in `boyko_ecs` (`vm.rs:85`, `:109`, `:199`) and its unix arm uses `libc` (`vm.rs:149`). Moving it down needs either a third-party dep (forbidden) or a **second** hand-declared per-OS backing, against `vm.rs:12-18`'s "These cfg arms are THE per-OS backing implementation for the whole engine". Inventing memory backing twice is a worse Principle-0 breach than the one this crate fixes. Argued in full in [`04-STORAGE.md`](04-STORAGE.md) |
| Any `boyko-####` literal, `emit_diag`, any print, any panic hook | The leaf is mute. A leaf that needs a diagnostic channel is the one edge that closes the cycle (§6) |
| Any thread, file, socket, syscall, `core::fmt` | Both consumers own their own I/O; a sink in the leaf would make `boyko_utils`-level crates carry a thread |
| `LogTarget` / `CONTROL` / the level model | Logging's taxonomy. The profiler has `ARM_MASK` and wants no levels; sharing would force one to carry the other's gate |
| Statistics: median, p95, band, `Floor`, `resolve` | Need the store, which is a `Resource` |
| `BitSet` / `SparseMap` / `TypeIntern` | Stay in `boyko_utils`. `boyko_diag` must not become a second utils |
| `ZoneId` semantics for the logger | The logger never names `ZoneId`; layer B is hosted, not exported to it (§5) |

**Growth rule** (verbatim from the decision record):

> A thing enters `boyko_diag` only if **both** subsystems *write* it **and** a disagreement
> between two copies would be observable in an artifact a reader joins. Anything one writes and
> the other only reads stays with the writer, behind a getter.

Applied at review time, the rule is a two-question checklist and **both answers must be yes**. A
proposal that fails it does not become a `boyko_diag` module with a comment explaining the
exception; **it stays where it was.**

---

## 5. Layer B: `profiling_abi` is HOSTED here, not shared

`profiling_abi` is written by the profiler and read by nobody else. By §4's growth rule it does
not qualify as shared, and it is not shared — it is **hosted**.

Its contents: `channel`, `scope`, `zone`, `dyn_registry`, `sample`, `lane_ring`, `macros`,
`tier`; `ARM_MASK`, `REGISTRY`, `ZoneLane`, `declare_zone!`, `counter!`, `gauge!`. It indexes
`lane()` ([`02-LANE.md`](02-LANE.md)), stamps with `ticks()` ([`01-CLOCK.md`](01-CLOCK.md)) and
counts through the loss vocabulary ([`03-LOSS.md`](03-LOSS.md)). It emits **no** code — every
`W92xx` condition is a `raise(DiagFlag::…)` plus a counter, read and emitted by
`boyko_ecs::…::profiling::fold`.

**The graph reason, stated plainly.** The ABI must sit below `boyko_threadpool` and
`boyko_rhi_vulkan`, because both open zones. Before this plan there was exactly one crate below
everything — `boyko_utils` — and the profiling plan put the ABI there for that reason. Two facts
close that option:

1. `boyko_utils` must keep an empty `[dependencies]` (verified `Cargo.toml:6`, table present
   with no entries), and the `profiling_abi` needs A1/A2/A3, so hosting it in `boyko_utils`
   would drag the substrate in behind it and end the zero-dep property.
2. `boyko_diag` is now the bottom. Hosting the ABI there costs one module in a crate the
   profiler already depends on, and costs the logger nothing at all.

**The logger never names it.** No `boyko_log` item refers to `ZoneId`, `ZoneLane`, `ARM_MASK`,
`declare_zone!`, `counter!` or `gauge!`. Layer B is `pub` from `boyko_diag` because Rust has no
finer visibility across crates, not because it is a shared surface; the constraint is enforced
by gate **DG10** (a grep gate over `crates/boyko_log/src`), not by the type system.

**Consequence to accept:** `boyko_diag`'s public API is larger than its shared surface. A reader
who takes "everything public in the bottom crate is shared" as a rule will be wrong. The module
is therefore named `profiling_abi`, not `abi`, and its module doc opens by saying it is hosted.

Layer B lands at profiling rung P1, **not** in D0/D1. **Its contents are specified by the
profiling plan and are not restated, cited or depended on here**; this file owns only its
*address* — that `boyko_diag` is where it sits, and the graph reason why. Nothing in the
substrate reads layer B, so nothing in the substrate changes if its contents change.

### 5a. A SECOND hosted module — `telemetry`, at profiling rung 13

The telemetry **wire format** is hosted here on exactly the same terms, and it is recorded so that
"hosted" stays a door with a lock on it rather than a precedent anyone can walk through.

It is **not shared**: the profiler writes it and one tool reads it, so §4's two-question checklist
refuses it as a shared primitive, and it enters — like `profiling_abi` — through the graph.

**The graph reason, MEASURED** (`cargo tree --edges normal`, at that rung):

| A `prof_decode` rooted at | Crates it must build |
|---|---|
| `boyko_diag` | **2** |
| `boyko_ecs` | 12 |
| `boyko_app` — where the WRITER lives | **45** |

The decoder is a leaf binary whose whole job is to read a file and print a table. Rooting it at the
writer's crate makes it build the Vulkan FFI, every shader and the render stack, and inherit that
stack's build state — which at rung 13 included a feature leg that did not compile.

**It does not touch §6.** Encoding is into a caller-supplied `&mut [u8]` and decoding is over a
`&[u8]`: no file, no print, no thread, no `core::fmt`. `std::fs` and stdout live in
`tools/prof_decode`, which is a separate crate for precisely that reason rather than a `[[bin]]`
here.

**Consequence, and the same one as above:** two of `boyko_diag`'s public modules are hosted rather
than shared. The count is **two**, and a third needs its own measured graph argument in this
section — not an appeal to these.

---

## 6. The mute-leaf rule

**`boyko_diag` emits no `boyko-####` code, prints nothing, installs no hook, opens no file,
spawns no thread, and does not use `core::fmt`.**

A leaf that needs a diagnostic channel is the one edge that closes the cycle: `boyko_diag` sits
below `boyko_log`, so it cannot call it, and if it could, the graph would have a cycle.
The consequence is that a condition **observed** in the leaf must be **reported** above it.

**The mechanism** is `DiagFlag` + `raise` / `take_raised` ([`03-LOSS.md`](03-LOSS.md)):

1. The leaf observes the condition — lane exhaustion, an uncalibrated clock read, a forward
   jump, a refused claim.
2. It calls `raise(DiagFlag::X)` (one `fetch_or`, `Release`) and increments the matching
   `LossCell` counter. It does **not** format, print, or name a code.
3. An emitter above the leaf — `boyko_ecs::…::profiling::fold` for `W92xx`, `boyko_log`'s own
   drain for its codes — calls `take_raised()` at its next fold, maps each set bit to a
   registry code, and emits it with the counter's value.

**What this costs, stated: a condition is reported at the next fold, not at the instant it
occurs.** The delay is one frame in the steady state. Three consequences follow and none is
hidden:

- A condition raised **before** any emitter exists is still reported. `DIAG_FLAGS` is `.bss`, so
  it is live from process start; a `W9201` raised at `ScheduleBuilder::try_build` before
  `LogPlugin::build` runs is emitted at the first frame's fold. This is strictly better than
  "boot the logger earlier", which is unenforceable across every host.
- A condition raised **after** the last fold — during teardown, or on the crash path after the
  drain — is **not reported at all**. The bit is set in a static nobody reads again. Named here
  rather than discovered later.
- The flag is a **bit**, not a count: N occurrences of one condition raise one bit. The count
  lives in the paired `LossCell`, and an emitter that prints the code without the counter is
  reporting "it happened" when it could report "it happened N times". Every `DiagFlag` therefore
  has exactly one paired counter, and **that pairing is a table in the emitter, not a
  convention** — the table was open question Q4 and is **RESOLVED at profiling rung 2**, in
  [`03-LOSS.md`](03-LOSS.md).

**Enforcement** is gate **DG9** (grep + the three `deny(clippy::print_*)` lints at the crate
root, §3) and gate **DG10** (`boyko_log` never names layer B). Both have showable REDs in
[`05-LADDER-GATES.md`](05-LADDER-GATES.md).

**What the rule does not claim.** It does **not** claim `core::fmt` is absent from the linked
symbol graph. `panic!`, `expect`, and slice bounds checks pull `core::fmt::Arguments` machinery
in regardless of anything this crate writes. The claim is about *diagnostic emission*: no format
string, no `Display`/`Debug` impl, no write to any stream. **The symbol-level claim is UNPROVEN
and is not made** (DG9).
