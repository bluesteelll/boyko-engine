# The seam — decisions neither subsystem may restate

<!-- CONTRACT
provides: seam/decisions-s1-s12
provides: seam/build-axis
provides: seam/free-when-off
provides: seam/joint-cost
provides: seam/landing-order
provides: seam/vocabulary
provides: seam/diagnostic-code-space
provides: seam/lifecycle-order
exports:  seam/open-owner-calls
assumes:  substrate/dedup-rationale
assumes:  substrate/crate-graph
assumes:  substrate/mute-leaf-rule
assumes:  substrate/clock-source
assumes:  substrate/lane-registry
assumes:  substrate/loss-vocabulary
assumes:  substrate/loss-fold
assumes:  substrate/never-freed-storage
assumes:  substrate/section-report
-->

*Carved from `docs/LOGGING-SYSTEM-PLAN.md` §Seam disposition (S1-S12), §Decision 25, §Decision 11,
§Decision 12's lifecycle block, §Invariant 2's stdout/stderr rule, §Implementation plan's
cross-plan preconditions, §Metrics' `config_tag` and P1 paragraphs, §Behaviour changes' `boyko_app`
and `boyko_threadpool` rows; `docs/PROFILING-SYSTEM-PLAN.md` §D21, §Invariant 8, §Implementation
plan's cross-plan prerequisites and rungs 14/16, §Metrics' `config_tag` paragraph, §Integration's
S6 obligation, §Performance budgets' two joint rows, §Open — needs the OWNER, and the inline (S..)
annotations on D2/D6/D19/D24; `docs/DIAGNOSTICS-SUBSTRATE-PLAN.md` §1's joint cost table.
**§S13 is new at the split** and has no counterpart in any monolith.*

---

## What this file is, and why it exists

`docs/PROFILING-SYSTEM-PLAN.md` and `docs/LOGGING-SYSTEM-PLAN.md` were reviewed twice each and
both passed. Held side by side they asserted **contradictory facts about the same object**, and
underneath that each had independently invented the same four primitives with incompatible
semantics. The round-3 seam review returned **INCOMPATIBLE AS WRITTEN**, findings **S1-S12**.

Those twelve decisions are recorded here **once**, with both plans' dispositions merged into one
row each. That is the whole point of this file: after the split, the pair can no longer disagree
about what was decided, because there is only one place where it is written.

**These decisions are MADE. They are not re-litigated here or anywhere downstream.** What a
subsystem file may do is state where a decision lands in its own design; what it may not do is
restate the decision, because two statements of one decision is the defect this corpus exists to
prevent.

One decision here is not from the review: **S13**, the owner's "free when not enabled"
requirement, folded in when this corpus was split. It is numbered beside the others so it is
discoverable, and its provenance is stated in its own section so a reader of the round-3 review is
not confused by its absence there.

---

## The twelve seam decisions

### S1 — The profiler owns the measurement channel end to end; `report!` is deleted

**DECISION.** The measurement output channel belongs to the profiling plan in its entirety.
`report!` is deleted from the logging design, and **nothing in the engine writes stdout at all**.
The six files that consume the `VB-P1d …` / `VB-P4 pass=…` / `VB-P4 regime …` stdout lines migrate
to the profiler's artifact in one commit at **profiling rung 7**.

**Because.** Both plans claimed the same channel. The printed measurement lines are a *measurement
contract* — a gate parses them and a floor is derived from them — and putting a measurement
contract behind a transport that has its own drop policy, its own rate policy and its own
sampling means a bench can silently lose the number it exists to produce. The producers
(`runner.rs:3089`, `:3096`, `:3121`, `:3137`) and the consumers move together or not at all.

**Cost to the losing side (logging).** `report!` is struck from the public API, from
`sync_out.rs`'s file list, from the `OUT_LOCK` row of the multithreading model, from the migration
ledger and from the behaviour table. **Mandatory test 16 is deleted and its number is not reused.**
L8b's 20 measurement rows drop to **zero**, and the migration denominator falls from ≤ 98 to
**≤ 78**. Open question 1 is struck. L8b is no longer free-standing: it waits on profiling rungs 7
and 7b. **`OUT_LOCK` survives** — deleting `report!` does not delete the lock, because
`write_oracle_line`, the sync-routed targets and `SINK_REQ` are all still callers, so Decision 9c
and G18 stand whole with their seven remaining callers enumerated.

**Cost to the winning side (profiling).** Rung 7 is the ladder's single subtractive rung and it
**breaks rung 8's input**: `vg_decidability_floor.rs` parses the shipped bench's own stdout
(`:133-160`) and it is the instrument that produces the `Floor` the band consumes. Every published
floor number is invalidated until **rung 7b** re-measures on the artifact channel. No new mechanism
enforces that — the new channel carries a new `WorkloadTag`, `Floor::from_session_file` is the only
constructor, and `resolve` already refuses a mismatched tag with `FloorWorkloadMismatch`.

**RED.** Leave one `println!("VB-P4 pass=…")` in `runner.rs`: caught by profiling's **G24** grep
(`rg 'VB-P1d |VB-P4 pass=|VB-P4 regime|VB-SV0-S1\.5 ' crates/*/src` must return zero) **and again**
by logging's `print_census.rs`. Reverse RED: point a migrated consumer at a **stale** artifact ⇒
the header's `build_hash`/`SessionId` mismatch makes the reader refuse rather than parse.

---

### S2 — `boyko_diag` is the new bottom; `boyko_utils` keeps zero dependencies

**DECISION.** A new zero-dependency crate `crates/boyko_diag` becomes the bottom of the workspace
graph and hosts the shared primitives. `boyko_utils` keeps its **empty `[dependencies]`** and does
**not** gain `boyko_log`. The profiling plan's planned `boyko_rhi_vulkan -> boyko_utils` and
`boyko_threadpool -> boyko_utils` edges are **withdrawn** and replaced by `-> boyko_diag`.

**Because.** This is the contradiction that started the review. Profiling justified moving its ABI
into `boyko_utils` *because that crate has zero dependencies*; logging's Decision 15 stated flatly
that "`boyko_utils` depends on `boyko_log`, not the reverse". Both cannot be true.
**Verified against the tree: `crates/boyko_utils/Cargo.toml:6` is an empty `[dependencies]`
section.** Profiling was right about the fact and wrong about the destination — `boyko_utils` is
a general-purpose leaf, and growing a diagnostics ABI inside it is the same accretion defect one
level down. Logging's sentence is **struck**; nothing in `boyko_utils` logs, and its four modules
are `bit_mask`, `identifiers`, `sparse_map`, `type_intern`.

**Cost to the losing side.** A new crate in the workspace, and every crate that logs takes a
transitive edge onto `boyko_diag`. That is the honest floor of the system existing, and it is
stated rather than smoothed: **no plan may present the substrate as free to depend on.**

**RED.** Logging's test 33 asserts the manifests: add `boyko_log` to `boyko_utils`'s
`[dependencies]` ⇒ red. Acyclicity itself is argued in `substrate/crate-graph` — `boyko_diag` has
out-degree 0, so no cycle can pass through it, and the same argument covers every later
`X -> boyko_diag` edge either plan adds.

---

### S3 — ONE lane registry in `boyko_diag`, with a deterministic worker-anchored topology

**DECISION.** One lane registry, owned by `boyko_diag`, anchored on the thread pool's own worker
ids: `LANE_WORKER_MAX = 64` / `LANE_DISPATCHER = 64` / `LANE_HOST = 65` / `LANE_SPARE_BASE = 66`.
A worker carries **one integer in both artifacts**.

**Because.** With two registries the same worker is lane 5 to the profiler and lane 37 to the
logger, so no reader can place a log line inside the zone it happened in — **the one joint question
the pair exists to answer becomes unanswerable by construction.** The topology is anchored rather
than hashed because worker ids are dense by construction: `MAX_WORKERS = 64` is unconditional
(`crates/boyko_threadpool/src/thread_pool.rs:49`, verified), the requested count is clamped to
`[1, MAX_WORKERS]` at `:554`, and the spawn loop enumerates at **`:602`** — not the `:601` the seam
record and the substrate plan both print. Re-read in the tree this session: `:600` is
`let stack_size = self.stack_size;`, `:601` is **blank**, and `:602` is
`for (worker_id, deque) in deques.into_iter().enumerate() {`, which is the file's only `enumerate()`.

**Cost to the losing side (logging).** S3 **deletes five things** from `boyko_log`: `MAX_LANES`,
the `hash(thread_id)` claim scan, the `owner` field, `MY_LANE`, and the `Drop`-carrying TLS guard.
Algorithms B collapses to a TLS read. Two further costs are stated rather than hidden: the claim
path is a load-then-CAS over the spares **in index order**, so concurrent claimants can **convoy**
on the first free slot — bounded at 14 CAS attempts, on a `#[cold]` path taken once per thread; and
a thread that never calls `release_lane()` holds its spare for the process — bounded at
14 × `LANE_BYTES` = **224 KiB**, counted as `lanes_leaked` and printed in the census.
One cost lands on *both* sides: because `boyko_diag` sits **below** `boyko_threadpool` it cannot
call `current_worker_id()`, so a worker thread holds **two** TLS `Cell`s after D1 — the pool's own
worker id and `boyko_diag::LANE`.

**Cost to the winning side (profiling).** It may not reuse
`current_worker_id_or_dispatcher_lane()`, which maps `WORKER_ID_UNATTACHED` onto lane **0**
(`crates/boyko_threadpool/src/tls.rs:69-78`) — the window thread, the present thread, the driver
callback thread and every test-harness thread all land there, which would make lane 0 MPSC and
destroy the SPSC property both transports rest on.

**A correction that belongs here, because both plans carry the wrong number.** Both monoliths say
`set_lane` lands at its **"two existing sites"** — profiling in its cross-plan prerequisites,
logging in its behaviour-change ledger. **There are three**, verified in the tree this session:
`worker.rs:24`, `thread_pool.rs:190` (`PoolInner::install`) and **`thread_pool.rs:279`
(`InstallGuard::drop`)**. The third is the load-bearing one — it covers the **unwinding** path,
and without it a panicking dispatcher stays `LANE_DISPATCHER` for the rest of the process. The
site table is owned by `substrate/lane-write-sites`; the count is corrected here because this is
where both plans state it.

**RED.** The **JOIN red**: one `warn!` and one `zone!` on the same worker must carry the **same**
integer in the two artifacts. It is deliberately **deferred** to a rung where it can fail —
logging **L5** / profiling **P2** — because before both transports exist there is no second
artifact to disagree with, and a gate that cannot fail is the defect this campaign has now caught
nine times.

---

### S4 — ONE clock in `boyko_diag`; `W0101` is deleted in favour of `W9207`

**DECISION.** `crates/boyko_log/src/tsc.rs` is deleted. Both subsystems consume
`boyko_diag::clock`: `ticks()`, `ticks_per_ns()`, `clock_epoch()`, `calibrate()`,
`note_forward_jump()`, `invariant_tsc()`, plus the 128-bit `SessionId` minted once and carried in
**both** artifact headers. Records store **raw ticks**. The single invariant-TSC diagnostic is
**`boyko-W9207`**; logging's `W0101` is **struck**.

**Because — and the benefit is AGREEMENT, not speed.** The boot saving is about one `cpuid`, not
20 ms, and this corpus says so rather than claiming a speedup. What one owner actually buys is
this: without it, a **suspend/resume** produces a profiler window quarantined as an epoch break
and, in the same seconds, log lines whose printed wall times are wrong by the suspend duration
**with no marker** — two artifacts that disagree, neither of which says why. With one owner, CPU ↔
log-record correlation is *exact*: same counter, same scale, same epoch, so a profiler sample and a
log record sit on one axis with no fitted offset. That is the only cross-domain correlation either
subsystem offers in v1, and it exists *because* the clock is shared. (GPU ↔ host correlation stays
out of scope: per Khronos, device timestamps "cannot be compared even across separate submits
within the same run" without `VK_EXT_calibrated_timestamps`.)

**One code, not two, because two answers to one question is the disease.** An uncontrolled code in
one plan and a controlled one in the other, for one condition, is exactly what S6 exists to stop.
`W0101` also had **no reachable red state** on any targeted machine by logging's own N30.

**Cost to the losing side (logging).** `tsc.rs` is deleted; `W0101` is struck from Decision 11,
from the gates' "no control possible" paragraph and from `untested_codes.txt`. **The header does
not grow**: `clock_epoch_lo: u8` spends v3's `_pad`, so the `HEADER_BYTES == 20` const assert
stands. S4 deferred the 4-byte-vs-4-bit choice to L3, and L3 takes the pad byte — eight bits
suffice because the sink is at most one park interval behind the producer, so at most one epoch
boundary can lie between them and the sink reconstructs the full `u32` by comparing against the
current `clock_epoch()`.

**RED.** Profiling's **G21, asserted on BOTH artifacts.** Inject a synthetic forward jump of 10 s
into `boyko_diag::clock` ⇒ `clock_epoch_breaks == 1`, the in-flight window is discarded, `W9216`
is emitted, the *next* window is complete, no `FrameRecord` carries a duration above
`MAX_PLAUSIBLE_FRAME_TICKS`, **and** every log record emitted after the jump carries the
incremented `clock_epoch`. Remove the detector ⇒ a 10 s interval appears in `max` and in p95 ⇒ red.
**Give the logger its own `ticks_per_ns` back ⇒ its rendered wall times drift by the injected
amount while the profiler's window is quarantined ⇒ the cross-check reds.** That second clause is
the one that could not exist before S4.

---

### S5 — `boyko_app` owns the WHOLE lifecycle; `PRE_FLUSH`; `sink_can_accept()`

**DECISION.** Nobody but `boyko_app` may boot or shut down either subsystem. One process-global
panic hook, owned by the logging plan; the profiler **registers** a callback and installs nothing.
Full mechanism in §*S5 expanded — the lifecycle order* below.

**Because.** Two subsystems each installing a process-global panic hook is a race over a single OS
resource whose winner is link-order-dependent, and a teardown in which each flushes itself has no
defined order — which is how GPU-side diagnostics came to be emitted **after** the logger had
stopped accepting them.

**Cost to the losing side (profiling).** `Profiler::arm()` does not own its own hook and cannot
assume one. `flush_on_panic` takes **no arguments and must not touch the `World`**, which forces
the telemetry double buffer and its file handle out of the `Profiler` `Resource` and into a
`boyko_app::profiling::stream` process-static. Eight `PRE_FLUSH` slots is a hard cap.

**Cost to the losing side (logging).** It owns a callback array it does not itself use, and a
registrant's bound — "no allocation, no lock, one `write_all`" — is asserted **per registrant** and
is **not provable in general**. That limitation is written into profiling's G15 "cannot claim"
column rather than left implicit.

**Divergence, recorded.** The seam record illustrated the ninth-registration failure as `E0110`.
That number is taken (`W0110` is `OUT_LOCK`'s steal code, `DIAGNOSTICS` is dense with
`index == code_idx`, and registry check 1 asserts numbers strictly increasing — two rows numbered
110 would not compile). The code is **`E0118`**, the next free slot in the `01xx` band.

**RED (four, at logging's L3-gate).** (a) `warn!` before `enable()` **with a synchronous
destination configured** ⇒ the bytes appear on it; restore the `.bss`-zero drop ⇒ no bytes ⇒ red.
*(The boundary is `enable()`, not `boot()`, and the qualifier is not decoration: S13 moves the
destination open onto the enable path, so before `enable()` there is no destination for the bytes
to appear on and the RED as first written — "before `boot()` ⇒ the bytes appear" — asserts a
positive outcome at an instant when no `LogConfig` has been handed over at all.)*
(b) a severe record after `shutdown()` ⇒ same. (c) a registered `PRE_FLUSH` callback sets a flag;
panic ⇒ the flag is set **and it ran before the crash drain**; move the call after the drain ⇒ the
ordering assertion reds.
(d) a deferred `DiagFlag` raised pre-boot appears in frame 1's output; delete the sticky flag ⇒
absent ⇒ red.

---

### S6 — The `92xx` block is reserved by the LOGGING registry as `Pending` rows

**DECISION.** The profiler has **no code registry of its own**. Logging's `codes!` registry seeds
block `92xx` at **L2** as `Pending(<profiling rung>)` rows, and check 2 (a doc page must exist) is
narrowed to `Live` rows only. Full mechanism in §*S6 expanded — the diagnostic code space* below.

**Because.** Logging's check 4 scans `docs/**.md` for `boyko-####` literals. The already-committed
profiling plan contains the literals `boyko-W9207` and `boyko-E9204`, so **without the reservation
L2 reds on a document nobody is editing.** And without the `Live` narrowing, L2 would owe eighteen
doc pages for codes with no emitters — **doc-rot manufactured by a gate.**

**Cost to the losing side (profiling).** Every rung that introduces a code carries **three explicit
line items**, not one: flip its registry row `Pending → Live`, add `docs/diagnostics/<code>.md`,
and land **one test that observes the code being emitted**.

**RED.** Write a `92xx` literal into any scanned document without its registry row ⇒ check 4 reds.
Narrow check 2 to *all* rows instead of `Live` ⇒ L2 owes eighteen pages it cannot honestly write ⇒
the rung cannot commit alone.

---

### S7 — One rule each for stdout, stderr and files

**DECISION.**

- **stdout** is written by exactly one thing in the whole workspace: `boyko_shaderdsl`'s CLI bins
  (the 58 allowlisted sites). **Nothing in the engine, the logger or the profiler writes stdout,
  ever.** The profiler's report has no console form at all.
- **stderr** is written by exactly two, and both go through `std::io::stderr()`'s **own handle**:
  the Vulkan validation messenger (`crates/boyko_rhi_vulkan/src/debug.rs:114`, untouched
  byte-for-byte) and `boyko_log::write_oracle_line`.
- **files** are the logger's sinks and the profiler's artifact/stream, each with a named owner.

**Because.** Sharing stderr's *handle* — rather than taking a raw fd — is what makes the two
producers share stderr's inner lock, so **neither can splice a line into the other**. That is what
keeps `scripts/golden.ps1:226`'s line-start match on `[vk-validation] ` working. *Ordering* between
the two producers remains undefined and is stated as such; **line integrity, not ordering, is what
the gate consumes.**

**Cost to the losing side.** The profiler gives up a console form entirely — a developer who wants
a number reads the artifact, not a terminal. `write_oracle_line` gives up raw-fd performance for
handle sharing.

**RED.** 200 `warn!` while the validation callback fires under `cmd /c … > f 2>&1`; every
`[vk-validation] ` occurrence must start a line. **Give `write_oracle_line` a raw fd ⇒ it splices ⇒
red.** (Logging L3-gate; it is test 16's replacement.)

---

### S8 — ONE loss vocabulary, accumulating in `u64`, never saturating

**DECISION.** `LossClass` / `LossCell` / `LossTotal` / `LossStatus` and the sticky `DiagFlag`
mechanism live in `boyko_diag::loss` and are shared. Counters accumulate in **`u64` and never
saturate**. One `DiagCensus`.

**Because.** If each subsystem keeps its own accounting, the profiler reports its drops *through*
the logger — so **under load, precisely when profiler drops occur, the report of the loss is
dropped and counted as a *logger* loss.** Two counters double-count one event and no rule says
which is authoritative. Sharing removes that second-order defect **by construction**: a profiler
drop becomes a counter read, not a record that can itself be lost.

**Cost to the losing side (logging) — S8 reverses this plan's own X4.** X4 had rejected `u64`
("an 8-byte RMW is more expensive"). That does not survive: **`lock xadd` costs the same at 4 and
8 bytes**, and the lane-owned cell needs no RMW at all. So the saturating `u32` goes, and with it
the **`SATURATED(>= 4294967295)`** census token — a token a reader could never compare against
anything. Logging's Decision 5, Decision 17's status table, Decision 21's row, its data structures
(`LossCell`s replace the `AtomicU32` pair), **G11's subject** and E18 are all rewritten.

**A correctness point that must not be softened (F5).** A plain `u64` cell read across threads is
**UB regardless of what x86-64 does**, and Miri reports it. The cells are `AtomicU64` accessed
`Relaxed` by the owner: that lowers to the identical `mov` pair with **no `lock` prefix**, so the
performance argument survives verbatim while the language-level defect disappears.

**RED.** The shared fold-exactness gate — profiling **G4b** = logging **G11** = substrate **DG5**,
one gate for both subsystems: a live producer increments while the consumer folds; the total must
be exact. *(That gate cannot land until blocker **Q2** is answered — see `substrate/loss-fold`.
`fold_into`'s producer-side lost-update window is **not** closed by `fetch_sub(observed)`, and D0
ships `loss.rs` without `fold_into` until an architect resolves it.)*

---

### S9 — ONE compile axis for both subsystems: `BOYKO_PROFILE`

**DECISION.** One env var, `BOYKO_PROFILE`, read by exactly **one** `build.rs` in the workspace —
`crates/boyko_diag/build.rs`. Neither `crates/boyko_log/build.rs` nor `crates/boyko_ecs/build.rs`
is created. Full tables in §*S9 expanded — the one build axis* below.

**Because.** Rev 3 of profiling had a private axis (`BOYKO_PROFILING_TIER`, read by a new
`crates/boyko_ecs/build.rs`) and logging had another. Two axes over one binary means a
configuration in which the profiler is folded out and the logger is not — a state no one chose and
no gate covers — and it means two CI matrices whose legs do not correspond.

**Cost.** Changing the profile rebuilds the workspace from `boyko_diag` up. CI grows from 1 to
**5 full-workspace legs (4 net new)** — *shared between the plans, not per-plan*. ~12 KiB of dead
`.bss` remains for folded `ZoneHandle` statics. **That is the whole cost on this axis** — this
line also claimed a typo in a `Deep` zone name is invisible in the shipping leg, and it is not:
`zone!`'s expansion names the identifier in both the gate (`const { $h::TIER <= GLOBAL_TIER }`,
read from the `mod` companion `declare_zone!` emits — reading it through the handle static is
`E0080`, measured) and the guard body, so the typo is `E0425` in every feature-on profile. Invisibility is the FEATURE axis's cost, and
§S13 below states the token/codegen distinction that decides it.

**RED.** `BOYKO_PROFILE=shipping BOYKO_LOG_MAX_LEVEL=trace cargo build` must fail with a named
message; delete the `compile_error!` ⇒ it builds and the header prints a ceiling the profile does
not name ⇒ red.

---

### S10 — Both perturbation gates run in the both-present configuration; ONE final joint baseline

**DECISION.** Every regression baseline is stamped `config_tag = {profiler: bool, logger: bool}`.
A sitting whose tag differs from its baseline's returns `NotResolved{ConfigMismatch}` and records
**UNPROVEN rather than failing the rung**. Logging's P1 becomes a **2×2**. **J2**, the joint
baseline sitting, is a rung, and it is last.

**Because.** The joint hot-path working set is **7-8 cache lines** against logging's isolated 4. A
logger-on/off delta measured with the profiler absent is a delta **in a configuration a shipped
frame never runs**. A 1×2 P1 would have been *a correct measurement of the wrong thing* — the same
class of defect as measuring frame time on a FIFO-clamped channel, one level up.

**Cost.** Until J2 lands, **neither** the profiler's +25 % `zone_cost` gate **nor** logging's
`G10d`/`G12c` revert clauses may fail anything. Both record `UNPROVEN`. That is a real loss of
enforcement across most of both ladders, taken deliberately over the alternative — a false
regression that a rung is failed for.

**Arithmetic correction carried by S10.** The seam record's joint totals — `dev` 9.33 MiB and
retail 1.95 MiB — are rev 3's, and **all four halves have been recut since**: the profiler to
6.67 `dev` / 0.89 retail, the logger to 2.90 `dev` / 1.19 retail. Recomputed from this revision's
own halves, and from nothing else: `dev` = 6.67 + 2.90 = **≈ 9.57 MiB**, retail = 0.89 + 1.19 =
**≈ 2.08 MiB**. **The record's table is not wrong; it is one revision old on BOTH rows** — see
§*The joint cost* below, where the arithmetic is written out, including why the retail figure moved
by more than a recut explains.

**RED.** Hand a gate a baseline with a foreign tag ⇒ `NotResolved`, **rung not failed**. Remove the
tag check ⇒ an armed-with-logger sitting is compared against a logger-absent baseline and a **false
regression** appears ⇒ red.

---

### S11 — Vocabulary: one word, one meaning, across the whole corpus

**DECISION.** The renames each plan owes are tabled in §*S11 expanded — the vocabulary* below.

**Because.** Two subsystems that use `window`, `census`, `target`, `tier` and `epoch` for five
different pairs of things produce a corpus in which a reader who learnt a token in one artifact
mis-reads it in the other. This is the cheapest of the twelve decisions to implement and the one
whose absence does the most quiet damage.

**Cost.** Both plans rename; neither gets to keep its first choice everywhere. Profiling loses
`RecordCensus` and the word `target`; logging loses `Lane`, `MY_LANE`, `MAX_LANES`,
`CONTROL_EPOCH`, its own `SessionId` mint and the phrase "windowed frame time".

**RED.** Not a gate — a review property. The one mechanical half is that the *deleted* names
(`report!`, `W0101`, `tsc.rs`, `MY_LANE`) cannot survive: a grep for them after their rung is a
one-line check, and logging's `print_census.rs` covers `report!`'s half.

---

### S12 — ONE never-freed-storage policy

**DECISION**, stated as a rule and not as a per-plan exception plea:

> **Extent known at compile time ⇒ `.bss` static. Extent chosen at run time from config ⇒
> `VmReservation`, and the owner must therefore sit at or above `boyko_ecs`.**

`boyko_diag::section_report` is the **ONE** implementation of the `llvm-readobj`/`objdump` section
probe for both plans (substrate DG6, profiling G22a/G22b/G23a/G23b, logging G3).

**Because.** Two residency proofs over two statics with two demand-zero arguments means a toolchain
change reds one gate and not the other, and **the reader cannot tell which is authoritative**. And
the rule is not a softening of Principle 0: the owner's standing correction targets
`std::Vec`/`Box` — heap, growable, not address-stable. `.bss` is none of those. The boundary is
also **forced, not chosen**: `VmReservation` is `pub(crate)` (`vm.rs:85`) with a `libc::mmap` unix
arm, so a std-only zero-dep leaf could host it only by taking a third-party dep (forbidden) or by
minting a **second** hand-declared per-OS memory backing against `vm.rs`'s single-source-of-truth
clause — **inventing memory backing twice is a worse Principle-0 breach than the one this crate
fixes.**

**Cost.** `boyko_diag` cannot own any run-time-sized table, so anything sized from config lives at
or above `boyko_ecs` and the two subsystems each carry a split ownership story. Logging pays it
directly: `CONTROL` cannot be an ECS column at all (there is provably no `World` before `boot()`,
inside a driver callback, or inside a panic hook), so it gets no `Query`, no change detection and
no `EnableTag` — mitigated by `control_epoch`, an `O(1)` repaint signal a UI polls instead of
subscribing.

**RED.** A `#[test]` declaring a `.bss` array sized from a `ProfilerConfig` value must fail
`assert_bss_eligible` **at compile time**; remove the const-assert ⇒ it compiles ⇒ red. *(The
compile-fail red cannot be a `#[test]` and must be `trybuild` — see `substrate/04-STORAGE.md` F8.)*
And the limit that must never be exceeded: the probe proves **absence of raw data in the image**.
That the OS leaves those pages uncommitted until touched is **UNPROVEN and is not claimed.**

---

## S13 — Free when not enabled: two axes, and the honest cost of each

**Provenance.** S1-S12 come from the round-3 seam review. **S13 does not** — it is an OWNER
requirement folded in when this corpus was split, and it is numbered here so it is discoverable
beside the others. A reader of the round-3 review will not find it there; that is expected, not an
omission.

**The requirement, as stated:** *"Profiling and logging must be FREE when not enabled. At game
launch, flags turn them on; without the flags the features simply do not work and cost nothing."*

### The requirement names one thing; the design keeps two apart

The sentence conflates two mechanisms with different powers. Only one of them reaches literal zero,
and only the *other* one can serve a binary that has already shipped. Collapsing them is how a
design ends up claiming zero cost and then being unable to produce a log for a player who reports a
bug.

| Axis | What it decides | Set when | Reaches zero? | Mechanism |
|---|---|---|---|---|
| **COMPILE-TIME CEILING** | what may exist in this binary **at all** | at build time, per `BOYKO_PROFILE` (S9) | **YES** — no branch, no symbol, no `.bss` row | `GLOBAL_TIER` (profiling, D21) and `GLOBAL_CEILING` (logging, Decision 2/3), both `const`, both emitted by the one `crates/boyko_diag/build.rs` |
| **RUNTIME FLAG** | what is **ON right now**, out of what survived the ceiling | at launch, before the game loop | **NO** — see the per-site row below | `ARM_MASK` (profiling, D20) and `CONTROL[target]` (logging, Decision 14), both `.bss`, both zero by default |

- **The runtime axis defaults to OFF and the default is free of code.** `.bss` is zero, so
  `ARM_MASK == 0` and every `CONTROL` byte reads `Level::Off == 0`. Every gate fails without any
  initialiser having run. This is the property logging's Decision 3 already rests on; S13 does not
  invent it, it extends it to the profiler and makes it a joint obligation.
- **A shipping build drops `Trace`/`Deep` entirely.** The short-circuit `&&` over a `const false`
  deletes the arm **and its operands**. **The two axes are NOT proved the same way, and an earlier
  revision of this bullet claimed they were:**
  - The **FEATURE** axis deletes TOKENS, because `#[cfg]` runs before expansion. So
    `zone!(NEVER_DECLARED_IDENT)` compiling with the feature off and failing with it on is a
    sound two-sided test, and profiling **G1(a)** keeps that token form.
  - The **TIER** axis cannot be tested that way, and no gate may claim it is. A `macro_rules!`
    expansion is a function of its invocation token stream alone, so *"does the expansion name
    its argument"* is **uniform over every site in one build configuration** — a token test
    therefore cannot separate two sites at different tiers. `const { false } && …` deletes
    CODEGEN, not tokens; `if false { UNDECLARED; }` is still `E0425`. Profiling **G14(a)** is
    consequently a per-site-BY-CONSTRUCTION symbol census over a single-site fixture bin, not a
    token test, and logging **G16** was never a token test either — it is a four-sided **symbol**
    gate on `emit_impl` monomorphisation. This bullet previously cited both as proof of a test
    neither performs.
- **The runtime axis is the only thing that can be asked for after the binary shipped.** A player
  reports a bug, support asks them to relaunch with the flag, and *the same binary* produces a log.
  A design with only the compile axis answers that request with "ship them a different build",
  which for a released title is not an answer. This is the whole reason the runtime axis exists and
  pays a per-site cost.

### How the flag arrives — MEASURED, and it is not free of work

**Verified this session over `crates/`, `src/` and `scripts/`: `std::env::args` and `args_os`
appear ZERO times in the entire workspace.** There is no argument parser to extend. Every runtime
switch this engine has is an environment variable — **28 distinct `BOYKO_*` names** are read from
`crates/*/src` today (`BOYKO_DISABLE_VALIDATION`, `BOYKO_RENDER_PATH`, `BOYKO_VB_BENCH`,
`BOYKO_HOST_DUMP`, …). So `--profile` / `--log=debug` as written in the requirement names a
facility that **does not exist in this tree**, and the plans must pick a route rather than assume
the first one:

- **(a) Environment variables**, matching the 28 existing switches: `BOYKO_PROFILE_ON=1`,
  `BOYKO_LOG=debug`. Zero new mechanism, zero new parse surface, and setting an env var is
  something support desks already ask players to do.
- **(b) A real argv parser in `boyko_app`.** This is a NEW facility and would be the **first argv
  reader in the workspace**, so it must be specified rather than assumed: unknown-flag behaviour,
  precedence against the env vars that already exist, and the `--` convention. It is not free and
  it is not one line.

Both routes feed **exactly the same thing**: one call to `boyko_log::enable(spec)` and/or
`Profiler::arm(world, cfg)` before the game loop. The delivery mechanism is a SCOPE call recorded
in §*Open — needs the OWNER*; **the enable path below is unchanged either way**, so no rung is
blocked on the answer.

### What "free when off" honestly costs

Three rows, because there are three costs and three different things drive them down. **The word
"zero" is not used as a summary of this table, and no plan may quote it as one.**

| Cost | What it actually is when the flag is off | How it is driven down | Floor |
|---|---|---|---|
| **MEMORY** | `.bss` is demand-zero: an all-zero static is emitted by the linker with a virtual size and **no raw data**, so an untouched table costs **address space, not physical RAM**. This is true **ONLY IF BOOT DOES NOT TOUCH IT** — a single write to one lane buffer commits that page, and the property is gone for it. | Nothing is touched until the flag flips: no clock calibration, no sink thread, no panic hook, no lane-buffer initialisation, no `VmReservation` commit, no `RATE` traffic, no target registration. See the enable-path table below. | Address space equal to the profile's declared table extents. **Not zero, and not resident.** And the second half is **UNPROVEN and is not claimed**: `substrate/section-report` proves the bytes are absent from the *image*; it does not prove the loader leaves the pages uncommitted. The plans state exactly that and no more. |
| **BOOT WORK** | zero syscalls, zero threads, zero hooks, zero writes | Driven to zero **by the memory row's rule** — it is the same obligation seen from the time axis rather than the space axis. Everything that used to run at boot moved onto the enable path. | **Zero, and this one really is zero.** Observable: logging G2 leg (b)'s OS-thread-count probe (with its own control) and leg (c)'s behavioural panic-hook probe already know how to see it; S13 re-points both from the `off` *profile* to the flag-off *run*. |
| **PER-SITE INSTRUCTION** | One `.bss` load plus one branch, at every surviving site, in every frame, forever. Logging: one `Relaxed` `u8` load of `CONTROL[T::ID]` and a compare. Profiling: one `Acquire` load of `ARM_MASK` — a plain `mov` on x86-64, so the ordering costs nothing — and one `bt`, statically predicted not-taken. | **It is not driven down. A RUNTIME FLAG CANNOT REACH ZERO PER-SITE COST** — the flag has to be read in order to be a flag. **Only the COMPILE-TIME CEILING removes it**, by deleting the site and its operands. | ≤ 2 ns per profiling site, ≤ 3 ns per logging site, ≤ 4 ns for a `warn!`/`error!` site carrying `sink_can_accept()`. All three are budgeted rows with in-sitting controls, stated in each plan's own budget file; none is asserted. |

### The enable path — what moves onto it, and why that is where it belongs

Every one-time cost either subsystem has moves out of boot and onto `enable`, which runs **at
launch, before the first frame, on the host thread**, where a syscall and a 20 ms calibration
window are free of hot-path concerns *and* free of frame-time concerns. Nothing about the
mechanisms changes; only their call site does.

| Moved | From | To | Note |
|---|---|---|---|
| `boyko_diag::clock::calibrate()` — 16 probe pairs over `CALIB_WINDOW_MS = 20` | boot | whichever enable path runs first | Already idempotent and CAS-guarded (`substrate/clock-source`), so "whichever runs first" needs **no new mechanism**. **With both subsystems off, the clock is never calibrated and never read**, and `ticks_per_ns()`'s uncalibrated arm is never taken because nothing stamps. |
| The `VmReservation` reserve + commit + publish + `mem::forget` for the sample slab and the store columns | `Profiler::arm` | **unchanged — `arm` IS the enable path** | D8/D15 already put every allocation there and already run it outside any system, with `debug_assert!(!is_in_system_run())`. S13 changes nothing here; it is recorded so the carve does not "fix" a row that is already correct. |
| The logging sink thread (`SinkMode::Thread`) | `boyko_log::boot()` | `boyko_log::enable()` | **This is the change.** `boot()` becomes a pure struct-fill that spawns nothing and installs nothing. G2 leg (b)'s thread-count probe becomes a gate on the flag-off run, not only on the `off` build profile. |
| The process-global panic hook and the `PRE_FLUSH` registration | `boyko_log::boot()` | `boyko_log::enable()` | With the flag off there is no hook at all, which is exactly what G2 leg (c) observes behaviourally. The profiler's `flush_on_panic` registration rides the same move (§*S5 expanded*). |
| First touch of the `LOG_LANES` / `LANES` lane buffers | first emit | the enable path may pre-touch, or may leave it to first emit | Either is admissible and the choice is a rung decision. What is **NOT** admissible is touching them while the flag is off. |
| Dynamic-target interning, `RATE` slot minting, scope registration | boot / plugin build | enable, or first use after enable | All are `#[cold]`, all are already idempotent, and none may run under a flag that is off. |
| The `LogRing` / `LogCensus` `VmColumn::grow_to` — reserve + commit | `LogPlugin::build` | **first drain that carries a record**, under `log_drain_system`'s `ResMut` | **This row was missing, and its absence left the ONE boot-time reservation the corpus still had.** The destination is deliberately *not* `enable()`: `boyko_log` has no dependency edge to `boyko_ecs`, so it cannot reach an ECS resource — see `substrate/crate-graph`. It needs no move mechanism at all, because `VmColumn` is lazy **by construction** (`crates/boyko_ecs/src/ecs/memory/vm_column.rs:437-449`: `self.vm` is `None` until the first growth event and the reservation syscall is deferred to it). What had to go was an EXPLICIT pre-grow in `LogPlugin::build`, added to buy a soundness argument that `LogRing`'s clauses 1-2 already supply. **With the flag off, `log_drain_system` returns before touching anything**, and that early return is load-bearing rather than an optimisation: the system has THREE duties and only one of them consumes `ECS_HANDOFF`. The `TARGET_STATS` snapshot copies a `.bss` array and the per-frame `frame_epoch` record is written **by the drain itself, not by the emission path** — so without the early return, `LogRing`'s column grows on frame 1 with every `CONTROL` byte still `Off`, and S13's property is false one layer below where it looks true. Cost when off: one `Relaxed` load per frame in `Last`. |

### Consequence for the joint retail figure, stated where a reader meets the number

**The joint shipping figure stops being a RESIDENT cost and becomes ADDRESS SPACE touched only by a
player who asked for diagnostics.** Every sentence in either plan that states a footprint as a
resident cost is re-cut on this rule. **The rule is stated over row SHAPES rather than as a list of
file citations**, for two reasons: this file sits *below* both plans and may not depend on either
one's page layout, and a rule that enumerates the rows someone remembered can be satisfied by
re-cutting exactly those and no others. Whatever file it sits in:

- A row that says **"resident, armed"** keeps the word for the ARMED column only. Its flag-off
  column is address space and reads ~0.
- A row that states a fixed table budget — `claimed_lanes × 16 KiB` plus a per-profile `.bss`
  total, a per-profile reserved total, "typically 8-12 lanes ≈ 128-192 KiB" — is stating a
  **RESERVED** extent, not a resident one. With the flag off, `claimed_lanes == 0`, no buffer is
  touched, and resident is 0.
- The joint table below gains the column it was missing: **flag-off resident ≈ 0**, flag-on
  resident as tabled.
- A **dedup saving**, wherever one is quoted, is a statement about *reserved extents*: S13 neither
  changes it nor licenses anyone to "correct" it into a falsehood by applying this rule
  mechanically. That said, at THIS revision the joint table quantifies no saving at all — see
  §*The joint cost*, where the cell is **UNKNOWN** and why is written out — so there is presently no
  number here to re-cut, and a plan that quotes one is quoting rev-3 arithmetic.

### The off-cost is MEASURED, not asserted — gate `GJ1`

**Claim.** Turning the runtime flag off removes the subsystems' cost down to the per-site floor —
and the per-site floor is what only the compile ceiling can remove.

**Instrument.** Three legs, ONE sitting, ABBA-counterbalanced, with an interleaved zero control, on
the **headless schedule bench** (`crates/bench_bevy_vs_boyko`) — never on a windowed frame.
`VK_PRESENT_MODE_FIFO_KHR` is unconditional
(`crates/boyko_rhi_vulkan/src/present/swapchain.rs:199`, verified), so wall-clock frame time is
clamped at the refresh interval and the channel is *structurally incapable of responding*; that is
the F3 lesson from logging's P1, and repeating it here would produce a pre-determined verdict
rather than a measurement.

- **(A) FLAG ON** — profiler armed, logger enabled, at the shipping ceiling.
- **(B) FLAG OFF** — **the same binary**, same scene, flags absent. `ARM_MASK == 0`, every
  `CONTROL` byte `Off`.
- **(C) CONTROL LEG — the ceiling removed, only the runtime check left.** Built with the const
  ceiling forced permissive (`BOYKO_PROFILE=dev`, so `GLOBAL_TIER = Deep` and
  `GLOBAL_CEILING = Trace`) and the runtime flags **OFF**. This leg carries every site the shipping
  ceiling had deleted, each now paying exactly one `.bss` load and one predicted branch.

**Verdict** is `resolve`'s and is **not restated here**: GJ1 lands in the profiler's gate table
(§*Placement*) and takes whatever band and `NotResolved{reason}` discipline that machinery defines,
because two statements of one statistical rule is the defect this file exists to prevent. What the
seam specifies is the part that is joint — the legs, the sitting and the REDs. Reported as three
pairwise verdicts: (A vs B), (B vs C), and (A vs C).

**THE RED, and it is the entire reason leg C exists: if C does not resolve apart from B, the
instrument measured nothing.** The gate then reports `NOT RESOLVED (control inert)` and **the
free-when-off claim is recorded as UNPROVEN on this box — it is not restated.** A two-leg A/B alone
cannot tell "the flag is off" from "the sites were never compiled in", and that distinction is
precisely what the owner's requirement turns on. **Second showable RED:** delete the runtime gate
from the emission macros so that B becomes the same code as A — B collapses onto A and the (A vs B)
pair stops resolving. **Third:** move the sink-thread spawn back into `boyko_log::boot()` — G2 leg
(b)'s thread-count probe reds on the flag-off run while GJ1 itself may not move at all, which is
why the memory/boot claims are gated by G2 and G3 and **not** by GJ1.

**What GJ1 CANNOT claim.**

- It cannot claim a *frame* got faster. This box's own decidability floor is
  **6.3 / 14.3 / 4.7 / 13.5 %** across four runs of one protocol
  (`docs/VG-DECIDABILITY-FLOOR.md`), so a frame-time claim below roughly 15 % is undecidable here.
  GJ1 bounds **CPU schedule work at a stated profiler/logger state** and nothing else.
- It cannot claim the MEMORY row. `.bss` residency is proved by `substrate/section-report` —
  absence of raw data in the image — and whether the OS commits an untouched page is **UNPROVEN**
  and not asserted by anything in this corpus.
- It may not **fail** a rung before **J2**, the joint baseline sitting (§*The landing order*). A
  flag-off number taken without the other subsystem present is not a number about the both-present
  configuration; before J2 it records `UNPROVEN`, like every other regression gate in both plans.

**Placement.** GJ1 is specified here because it is a joint gate over both subsystems. It is added
to profiling's gate table and to logging's ladder, both assigned to the **J2** sitting.

### One thing this design refuses

A per-site runtime check that is "optimised away" — either by hoisting a global `AtomicBool` read
out of the loop, or by patching the call sites at enable time (a hot-patch / trampoline scheme).
Neither is taken. The first is not something the compiler may do across an opaque call, so it is a
**hope rather than a mechanism**. The second writes executable pages at run time, which is a new
capability class in a codebase whose emission path is *defined* by having no allocation, no lock
and no syscall. The honest statement is the one already in the table: **the branch stays, and the
compile ceiling is what deletes it.**

---

## The joint cost

Owned in **one** place, because it is inherently a joint number and three separate statements of it
is how the corpus contradicted itself. **Neither plan may quote its isolated figure as the shipped
one.**

**Every total in this table is the sum of its own printed operands, and where an operand does not
exist at this revision the cell says UNKNOWN rather than carrying a number that does not add up.**

| | Profiling | Logging | Naive sum (two independent implementations) | With `boyko_diag` | Flag OFF (S13) |
|---|---|---|---|---|---|
| **Total, dev** † | 6.67 MiB | 2.90 MiB | **UNKNOWN** | **≈ 9.57 MiB** reserved (= 6.67 + 2.90) | **≈ 0 resident** |
| **Total, shipping** † | 0.89 MiB | 1.19 MiB | **UNKNOWN** | **≈ 2.08 MiB** reserved (= 0.89 + 1.19) | **≈ 0 resident** |
| TLS slots (diagnostics) | 1 | 1 (+`Drop`) | 2 | **1**, no `Drop` — but a worker holds **two** `Cell`s, the pool's and `boyko_diag`'s | unchanged |
| `rdtsc` per {zone + log record} | 2 | 1 | 3 | **3 — sharing does NOT reduce it** | 0 (nothing stamps) |
| Allocations, first emit | 0 | ≤ 1 | ≤ 1 | **0** | 0 |
| Calibrations | 1 (20 ms) | 1 probe | 2 | **1** | **0** — moved to the enable path (S13) |
| **Joint hot-path working set** | 3-4 lines | ≤ 4 lines | — | **7-8 cache lines** | per-site load only |

**† The two Total rows' first two cells are this revision's WITH-SUBSTRATE halves, not "alone"
figures.** For every other row the first two columns really are the isolated figures — which is why
those rows' naive sums are computable and the Totals' are not. Rev 3 kept the two apart (it printed
a *logging alone* of 3.46 MiB `dev` beside a with-substrate 2.68); rev 4 recut only the
with-substrate halves and no "alone" figure survives anywhere in this corpus. The columns collapsed,
so **there is nothing to add**.

**The arithmetic, written out so a reader can check it rather than trust it.**

1. **The Totals are sums of the operands printed beside them, and of nothing else.**
   `dev` = 6.67 + 2.90 = **9.57 MiB**; in the KiB the source rows actually state,
   ≈ 6 831 + 2 972 = **9 803 KiB = 9.57 MiB**. `shipping` = 0.89 + 1.19 = **2.08 MiB**;
   908 + 1 220.26 = **2 128.26 KiB = 2.08 MiB**. The halves are this revision's: the profiler's
   sizing rows (≈ 908 KiB `shipping`, ≈ 6.67 MiB `dev`, both **with** its `.bss` statics counted)
   and the logger's `.bss` budget (1 220 KiB `shipping`, 2 972 KiB `dev`). `boyko_diag`'s own
   `.bss` is already inside them and is counted once — qualification (2) below.

   ⚠️ **The `shipping` operand was stale for one round, and the failure is worth keeping.** This
   table previously printed a logging half of 1 180 KiB and a total of 2.04 MiB. The total was a
   correct sum of its own printed operands — which is exactly why the error survived a repair pass
   whose stated job was to make every total equal its operands. The logger's own `.bss` budget row
   re-derived the column term by term (512 + 32 + 16 + 16 + 4.25 + 0.008 + 256 + 64 + 320 =
   **1 220.26 KiB**) and showed that **no subset of the table's rows sums to 1 180**, so 1 180 was
   never a different configuration — it was wrong. **A total that checks out against its operands
   proves nothing about the operands.** Any future edit to this row must re-read the source row it
   quotes, not merely re-add the numbers printed here.
2. **Why the naive-sum cell is UNKNOWN.** A naive sum is what two INDEPENDENT implementations would
   reserve, and rev 3 had those figures: its `dev` row checked out exactly, 6.65 alone + 3.46 alone
   = 10.11 naive against 6.65 + 2.68 = 9.33 with the substrate, a difference of 0.78 MiB. Rev 4
   recut **all four** halves — profiler 6.65 → 6.67 `dev` and 0.85 → 0.89 retail, logger 2.68 → 2.90
   `dev` and 1.10 → 1.15 retail — and recut only the with-substrate ones. **UNKNOWN is the honest
   cell**: it is not 10.11, which is rev-3 arithmetic over a "logging alone" operand nobody
   recomputed, and it is not the with-substrate total, which is the substitution that broke the
   retail row in the first place.
3. **What happened to the 9.58 and the 1.95 this file printed until now.** 9.58 came from patching
   rev 3's *total* — 9.33 + 0.25 for B2's `ECS_HANDOFF` — instead of re-adding the halves; but the
   logger's 2.90 already contains that 256 KiB, so the patch and the recut overlap and the sum of
   the printed halves is 9.57. **1.95 is the worse case: it never equalled the sum of its own
   operands in ANY revision.** Rev 3 printed `0.85 | 1.16 | naive 1.95` and 0.85 + 1.16 = **2.01**;
   this file printed `0.89 | 1.15 | naive 1.95` and 0.89 + 1.15 = **2.04**. 1.95 is 0.85 + 1.10 —
   rev 3's *with-substrate* halves — sitting in the naive column, which is also the whole source of
   the "zero saving" that column then claimed. **The joint retail figure at this revision is
   2.08 MiB**, and it is the number the owner call below is about.

**The substrate is bought for CORRECTNESS, not footprint** — one lane number, one epoch, a loss
report that cannot itself be dropped. **Neither subsystem plan may claim otherwise, and no rung of
any plan is justified by a byte count.** The saving this sentence used to quantify — 0.78 MiB in
`dev`, zero in `shipping` — is **UNKNOWN at this revision** and no plan may quote it: it is rev-3's
10.11 − 9.33, whose "logging alone" operand rev 4 never recomputed, and its `shipping` half never
held at all (the rev-3 row's own operands give 2.01 naive against 1.95 with the substrate, a saving
of 0.06 MiB, not zero). The rule stands without the number, which is the point — **it never rested
on it.** *(A statement about reserved extents; S13 does not touch it either way.)*

**Two qualifications, because the table is otherwise read as stronger than it is.** (1) The "1 TLS
slot" row counts *diagnostics* slots; `boyko_threadpool::tls::CURRENT_WORKER_ID` is untouched, so a
worker holds two `Cell`s after D1 — the second exists only because `boyko_diag` sits *below*
`boyko_threadpool` and cannot call `current_worker_id()`. (2) `boyko_diag`'s own `.bss` (≈ 42 KiB
dev) is attributed to its row **exactly once**; the joint table already counts those bytes inside
the two subsystems' rows, and double-counting them would **manufacture a footprint regression out
of a move**.

**The ≈ 2.08 MiB retail figure against the profiling plan's "≤ 1 MiB retail" headline is an
owner-facing VALUES question**, recorded in §*Open — needs the OWNER* below. **The correction makes
that question larger, not smaller**, which is why the number is carried into the open call rather
than left to be re-derived there: any document still stating the joint retail figure as 1.95 MiB is
quoting a cell that never summed.

---

## The landing order

Owned here so neither ladder can state it differently.

| Rung | Waits for | Because |
|---|---|---|
| logging **L0** | substrate **D0** (clock, lane, loss, storage policy, `section_report`) and **D1** (`boyko_threadpool -> boyko_diag`; `set_lane` at its **three** sites) | every lane index the crate uses is minted there (S3), and `GLOBAL_CEILING`/`LANE_COUNT` come from `boyko_diag/build.rs` (S9) |
| profiling **rung 1** | substrate **D0** and **D1** | same |
| profiling **rung 2** | logging **L3** (the sink, `flush`/`shutdown`, `write_oracle_line`, the panic-hook chain and `PRE_FLUSH`) | **the fold is what emits every `W92xx`** — `profiling_abi` emits nothing at all (S5/S6) |
| logging **L8b** | profiling **rung 7** (the six stdout consumers migrated) **and rung 7b** (floor re-measurement) | S1: L8b's 20 measurement rows **do not exist**, because rung 7 already removed their producers. Running L8b first would leave `report!`-shaped work with no macro to do it |
| **J1** | = profiling rung 14 **+** logging L17 | S9: **one compile axis cannot be split across two rungs**, and the 5 CI legs are built once |
| **J2** | last, after both subsystems are present | S10: whichever subsystem landed second must not be measured against a baseline taken without it |

**J2 in full.** The joint baseline sitting re-takes `zone_cost`, `fold_cost`, `P1`, `P2` and the
`log_*` benches **in the both-present configuration, in ONE sitting**, and stamps every baseline
file with `config_tag = {profiler, logger}`. **Gate `GJ1` (S13) is taken in this sitting.**

**S10's `config_tag` rule, stated once for both ladders.** A baseline file is stamped
`config_tag = {profiler: bool, logger: bool}`. A sitting whose tag differs returns
`NotResolved{ConfigMismatch}` through the existing `FloorWorkloadMismatch` path and **records
UNPROVEN rather than failing the rung**. **Until J2 lands, neither the profiler's +25 % `zone_cost`
gate nor logging's `G10d`/`G12c` revert clauses may fail anything.**

**RED for the rule itself.** Hand a gate a foreign-tag baseline ⇒ `NotResolved`, **rung not
failed**; remove the tag check ⇒ an armed-with-logger sitting is compared against a logger-absent
baseline and a **false regression** appears ⇒ red.

**S10's second half — the leg matrix.** Logging's P1 is a **2×2**: {logger off, on} × {profiler
absent, armed}, all four legs ABBA-counterbalanced in one sitting with the zero control
interleaved. Its claim is "logger-on vs off **at a fixed profiler state**", reported at both
states. **A 1×2 P1 would have been a correct measurement of the wrong thing**, because the joint
working set is 7-8 cache lines against the logger's isolated 4, so a delta measured with the
profiler absent is a delta in a configuration a shipped frame never runs.

---

## S9 expanded — the one build axis

**Exactly one `build.rs` in the workspace reads `BOYKO_PROFILE`: `crates/boyko_diag/build.rs`**, at
the bottom of the graph, so a change rebuilds every dependent. It emits `GLOBAL_TIER`,
`GLOBAL_CEILING`, `LANE_COUNT`, `REGION_CAPACITY`, `ENGINE_ZONE_SLOTS`, `MAX_USER_BUDGET`,
`DYN_NAME_BYTES` and `BOYKO_BUILD_HASH`, plus `cargo:rerun-if-env-changed`. **`crates/boyko_log/build.rs`
is NOT created, and neither is `crates/boyko_ecs/build.rs`** (rev 3's integration row is withdrawn);
both subsystems **re-export** their consts from `boyko_diag`.

**Verified this session: no `build.rs` exists in any workspace member or at the root.**
`crates/boyko_diag/build.rs` would therefore be the **first build script in this workspace**, which
is why it belongs to the joint rung **J1** and explicitly **not** to substrate rung D0.
**Also verified: `BOYKO_PROFILE` appears nowhere in `crates/`, `src/`, `scripts/`, `.github/` or
`docs/` outside the three plan documents — the name is free.**

| `BOYKO_PROFILE` | `GLOBAL_TIER` | `profiling-analysis` | log `GLOBAL_CEILING` | `LANE_COUNT` | `REGION_CAPACITY` | `ENGINE_ZONE_SLOTS` | `MAX_USER_BUDGET` | default `LogRuntimePreset` |
|---|---|---|---|---|---|---|---|---|
| `dev` (default) | `Deep` | on | `Trace` | 80 | 1024 | 4096 | 3072 | `Dev` |
| `editor` | `Dev` | on | `Debug` | 80 | 1024 | 4096 | 3072 | `Editor` |
| `shipping` | `Always` | off | `Info` | 32 | 128 | 256 | 512 | `Shipping` |
| `shipping-min` | `Always` | off | `Warn` | 32 | 128 | 256 | 512 | `ShippingMin` |
| `off` | feature `profiling` off | off | `Off` | 0 | — | — | — | `Off` |

- **`BOYKO_PROFILING_TIER` / `BOYKO_PROFILING_REGION_CAPACITY` / `BOYKO_PROFILING_DYN_CAP` /
  `BOYKO_LOG_MAX_LEVEL` survive ONLY under `BOYKO_PROFILE=custom`.** Setting one beside a named
  profile is a **`compile_error!`** naming the conflict.
- **The default is a default, not a coupling.** A `shipping` build may select
  `LogRuntimePreset::Dev` at run time, which is why the header prints **three independent facts** —
  `build_profile=… runtime_preset=… ceiling=…` — and not one profile name. The 128-bit
  `boyko_diag::SessionId` appears beside them, so an uploaded log and an uploaded artifact identify
  the same session.
- **The runtime axis is the logging plan's `LogRuntimePreset`** (five presets: sinks, rotation,
  sampling, sink mode, census policy) and it has **no `GLOBAL_CEILING` column** — a struct chosen by
  the host at run time cannot deliver a `const`. Its table is the logging plan's to state, and this
  file does not restate it.

**CI: 5 legs**, one per named profile, of which `dev` is the existing leg ⇒ **4 net new
full-workspace builds**, *shared between the plans, not per-plan*. `custom` is never built in CI.
G14/G16's cross-profile symbol censuses are CI **steps** consuming two legs' artifacts, not extra
legs. Each leg needs its own `CARGO_TARGET_DIR` with a size cap, under the standing "never two
bench jobs concurrently" rule — `target/` once reached 74 GB and took the disk to zero,
masquerading as mingw errors.

**REDs.** (1) `BOYKO_PROFILE=shipping BOYKO_LOG_MAX_LEVEL=trace cargo build` must fail with a named
message; delete the `compile_error!` ⇒ it builds and the header prints a ceiling the profile does
not name ⇒ red. (2) Assert the header carries all three fields and that `runtime_preset` and
`build_profile` **can differ** in one binary; print only one ⇒ red. (3) G16's two-sided symbol
gate: no `emit_impl` monomorphisation reachable from a `debug!`/`trace!` fixture may appear in the
`shipping` binary, **and it must appear in `dev`** — the fixture includes a `dyn_debug!` site,
because a dynamic site has no gate (a) and `GLOBAL_CEILING` is the only thing that deletes it.

---

## S11 expanded — the vocabulary

One word, one meaning, across the whole corpus.

| Word | Reserved for | The other thing is called | Owed by |
|---|---|---|---|
| `window` | the **statistics horizon** (`WINDOW` frames) | the OS object is `os_window`; frame time is **`presented`** frame time, never "windowed" | logging |
| `census` | `LogCensus` | the profiler's record-order witness is **`CommandWitness`**, not `RecordCensus` | profiling |
| `target` | logging's **sink type** (`LogTarget`) | profiling says **`budget`**, never "target" | profiling |
| `tier` | `ZoneTier` (`Always`/`Dev`/`Deep`) | retention is **`retention_tier`**, never bare `tier` | profiling |
| `clock_epoch` | `boyko_diag`'s discontinuity counter | **NOT** `CONTROL_EPOCH_CTR` (the control-change counter, accessor `control_epoch()`), **NOT** `FLUSH_SEQ`, **NOT** the per-frame `frame_epoch` record | logging |
| `Lane` | `boyko_diag`'s topology | logging's ring type is **`LogLane`**; `MY_LANE`/`MAX_LANES`/the claim scan become `boyko_diag::lane()` / `LANE_COUNT` | logging |
| profile | the **compile** axis (`BOYKO_PROFILE`) | the runtime one is **`LogRuntimePreset`** (v3's "`LogConfig` profile") | logging |
| `SessionId` | one mint in `boyko_diag` | no per-crate `session.rs` | both |

**Deleted names, which is the mechanically checkable half:** `report!` (S1), `W0101` (S4),
`tsc.rs` (S4), `MY_LANE` / `MAX_LANES` / the `hash(thread_id)` scan / the `owner` field / the
`Drop`-guard TLS (S3), `RecordCensus` (S11), `Floor::from_aa_control` (profiling D11),
`fmtv` (logging Decision 13), `LogFilter` (logging Decision 14), `__gpu_null` (profiling D5).

---

## S6 expanded — the diagnostic code space

**The profiler has no registry of its own.** Logging's `codes!` registry is the only one, and at
**L2** it seeds block **`92xx`** as `Pending(<profiling rung>)` rows.

**Why the reservation must happen at L2 and not later.** Registry **check 4** scans `docs/**.md`
for `boyko-####` literals. `docs/PROFILING-SYSTEM-PLAN.md` is already committed and already
contains **`boyko-W9207`** and **`boyko-E9204`**, so without the reservation L2 reds on a document
nobody is editing.

**Why check 2 is narrowed.** Check 2 ("a doc page must exist") applies to **`Live` rows only**.
Otherwise L2 would owe eighteen pages for codes with **no emitters** — doc-rot manufactured by a
gate. Statuses are `Pending` / `Live` / `Historical`.

**The per-rung obligation, which is what makes the narrowing safe.** **Every profiling rung that
introduces a code carries THREE explicit line items:** flip its registry row `Pending → Live`; add
`docs/diagnostics/<code>.md`; and land **one test that observes the code being emitted**.

**Block `92xx` — 18 codes.** `W9201` engine zone registry exhausted (*warning, not error* — C-III)
· `W9202` GPU pair budget exhausted · `W9203` region overflow / unclaimed drops · `E9204` profiler
already bound to another world · `W9205` zones LOST this window (once per window, with a count) ·
`W9206` contrast NOT RESOLVED · **`W9207` invariant TSC absent — the single invariant-TSC code for
both subsystems (S4)** · `W9208` engine registry ≥ 90 % · `W9209` late samples dropped · `W9210`
user zone budget or name arena exhausted · `W9211` fold working set exceeds L1d · `W9212`
`register_zone` refused an engine scope · `E9213` re-arm with a different geometry · `W9214`
telemetry path unwritable at boot · `W9215` telemetry write error, streaming disabled · `W9216`
clock epoch break, window discarded · `W9217` GPU slots abandoned at teardown · `W9218` telemetry
quantile subscription refused.

> **A live divergence between the two monoliths, recorded rather than silently resolved.**
> The profiling plan's §Integration says L2 seeds **"all 18 rows"** and lists eighteen codes;
> logging's S6 row and its L2 rung row both say **"17 `92xx` rows"**. Counting the profiling
> plan's own list gives **18**. Logging's number is one short. **The count is 18**, and whichever
> plan lands L2 must use it — a reservation one row short reds check 4 on exactly the code it
> forgot. *(This is precisely the class of defect the split exists to surface: two documents, one
> object, two numbers, three review rounds.)*

**`profiling_abi` emits NOTHING.** The leaf is diagnostically mute: every `W92xx` condition
observed below or before the logger is a `boyko_diag::loss::raise(DiagFlag::…)` plus a counter.
**`boyko_ecs::…::profiling::fold` is the only emitter**, reading `take_raised()` at the first fold
after boot. That is what makes a `W9201` refused at `ScheduleBuilder::try_build` — before
`LogPlugin::build` has run — **late rather than lost**. "Boot the logger earlier" is unenforceable
across every host and is not relied on.

**VERIFIED this session: no `92xx` literal exists in source today.** The source literals, counted
over `crates/*/src` as **`boyko-`-prefixed occurrences** — which is what check 4 matches — are
`boyko-B1802` ×24, `boyko-B0002` ×7, `boyko-B9001` ×6, `boyko-B9101` ×4, `boyko-B9005` ×3,
`boyko-B9004` ×3, `boyko-B9002` ×3, `boyko-B1801` ×2, `boyko-W1501` ×1.

> **The prefix is load-bearing in that sentence, and the census drifts without it.** A bare
> `[BWE][0-9]{4}` grep over the same corpus returns `B0002` **×17** and `W1501` **×3** — because
> the bare form also matches `#[should_panic(expected = …)]` substrings and prose — and it sweeps
> in rustc's own codes (`E0521`, `E0509`, `E0365`, `E0277`) that have nothing to do with this
> registry. This is the same trap logging's Goal section names for the print census: **a raw
> occurrence count is not a site count**, and a gate defined over the looser number can be driven
> green by editing a comment.

---

## S5 expanded — the lifecycle order

**`boyko_app` owns the WHOLE lifecycle and nobody else may boot or shut down.**

```
BOOT     boyko_log::boot(cfg)  ->  App::new  ->  LogPlugin::build (binds LogRing/LogCensus)
         ->  ProfilerPlugin::build
         ->  [flag ON:]  boyko_log::enable(spec)  ||  Profiler::arm()
                         [arm registers flush_on_panic in PRE_FLUSH]
FRAME    unchanged
TEARDOWN flush_gpu()  ->  Profiler::disarm()  ->  boyko_log::flush()  ->  boyko_log::shutdown()
```

**`flush_gpu` moves ahead of `flush`. That single reordering is the whole fix** for the teardown
hole where GPU-side diagnostics were emitted after the logger had stopped accepting them.

**`||` is deliberate.** The two enable paths are **unordered with respect to each other**; both are
taken only when the runtime flag is on; both run before the first frame; and `calibrate()` rides
whichever of them runs first, which needs no new mechanism because it is already idempotent and
CAS-guarded (`substrate/clock-source`).

*(Re-cut by S13: `boot(cfg)` becomes a pure struct-fill. The sink thread, the panic hook and the
`PRE_FLUSH` registration move to `boyko_log::enable()`, and `Profiler::arm()` — which was already
the profiler's enable path — is unchanged. **`enable(spec)` is DRAWN above, on the same footing as
`arm()`**, because S13 makes it the call that performs every syscall, spawns the thread and installs
the hook: an order-owner that omits it states an order for the calls that no longer do the work. The
RELATIVE order of the calls that were already drawn is unchanged.)*

**ONE process-global panic hook, owned by the logging plan.**

```rust
/// .bss, claimed by CAS, holding `extern "C" fn()`. Called by flush(), by
/// shutdown(), and by the panic hook at step 1.5 — BEFORE the crash drain.
static PRE_FLUSH: [AtomicPtr<()>; 8] = [const { AtomicPtr::new(null_mut()) }; 8];
pub fn register_pre_flush(f: extern "C" fn()) -> Result<(), PreFlushFull>;
```

`Profiler::arm()` **registers** `flush_on_panic` there and **installs no hook of its own**. A
registrant's contract — **no allocation, no lock, one `write_all`, and it must not touch the
`World`** — is asserted **per registrant** and is **not provable in general**; that limitation is
written into profiling's G15 "cannot claim" column. The no-`World` clause is what forces the
telemetry double buffer and its file handle out of the `Profiler` `Resource` and into a
`boyko_app::profiling::stream` process-static — consistent with S12's extent rule, since both
extents are compile-time constants. **Eight slots is a hard cap; a ninth registration returns `Err`
and emits `boyko-E0118`** (not `E0110` — see S5's divergence note).

**`sink_can_accept()` — one predicate closes both lifecycle holes.**

```rust
#[inline] fn sink_can_accept() -> bool;   // one Relaxed load of SINK_STATE
```

When it is **false** (`NotBooted` or `Exited`) **and** the level is `Warn`/`Error`, the record takes
the **synchronous** channel instead of being dropped. That closes the pre-`boot()` hole and the
symmetric post-`shutdown()` hole with one branch. Cost: one extra load plus a predicted-not-taken
branch on the **failed-gate path of `warn!`/`error!` only** — `info!`/`debug!`/`trace!` are
untouched, so the ≤ 3 ns row stands and the `log_disabled_warn ≤ 4 ns` row is what bounds the
addition.

**Deferred diagnostics.** Any condition observed *below* the logger or *before* it is
`boyko_diag::raise(DiagFlag)` plus a counter; `boyko_ecs`'s fold reads `take_raised()` at the first
drain after boot and emits the code then. So a profiling `W9201` refused before `LogPlugin::build`
is **not lost — it is emitted at frame 1**.

---

## Open — needs the OWNER, not the architect

Collected here so neither plan can bury one in a disposition table.

1. **VALUES — the shipping diagnostics budget is ≈ 2 MiB, not 1.** The profiling plan's headline is
   ≤ 1 MiB and it holds *for the profiler alone*. With `boyko_log` present the joint figure is
   **≈ 2.08 MiB** — 0.89 + 1.19, recomputed this revision from the two halves; the **1.95** carried
   here until now was rev 3's *with-substrate* halves (0.85 + 1.10) standing in a column that was
   supposed to hold the sum of the printed ones, and it equalled 0.89 + 1.15 no better than it
   equalled 0.85 + 1.16. The owner may have read the 1 MiB row as the whole diagnostics budget.
   Reducing it means cutting one of: logging's 32 × 16 KiB lanes (**512 KiB**), its `SINK_OUT`
   (**256 KiB**), or the profiler's non-foldable user-zone arenas (**40 KiB** in `shipping`).
   *(S13 changes what the number means — reserved address space, not resident RAM — but it does not
   answer the question, because the reservation is still declared.)*

2. **SCOPE — `shipping-min` semantics.** Logging's `shipping-min` disables the resident sink
   *thread*, but the profiler's `Always` tier **still writes a telemetry stream synchronously on the
   dispatcher** in that profile. A title that chose `shipping-min` to avoid a resident diagnostics
   thread **still pays a per-window `write_all`.** Keep, or make `shipping-min` also disable
   telemetry?

3. **SCOPE, new at the split — how the enable flag arrives.** `env::args`/`args_os` appear **zero**
   times in the whole workspace, so the requirement's `--profile` / `--log=debug` names a facility
   that does not exist here. **(a)** an env var matching the 28 existing `BOYKO_*` switches, or
   **(b)** a new argv parser in `boyko_app` that must be *specified* — unknown-flag behaviour,
   precedence against the env vars, the `--` convention — rather than assumed. **No rung is blocked
   on the answer**: both routes call the same `enable`.

4. **SCOPE — whether `boyko_demo` keeps a third-party log facade at all.** Its third-party logging
   is larger than the ledger says: two use sites plus `env_logger` and `console_log`, which exist
   only to service the `log` facade — and `log 0.4` is in the build graph **transitively** via
   `eframe`/`egui`/`naga`/`gpu-allocator`/`bevy_ecs` regardless. So the tidy check can only ever be
   about **DIRECT declarations**, and it must say so. *(Substrate Q5.)*

---

*Three blockers are open and are NOT owner calls — they are architect calls, and they travel with
their own files: `LANE_COUNT = 32` in shipping (`substrate/02-LANE.md`, Q1), `fold_into`'s
lost-update window (`substrate/03-LOSS.md`, Q2), and the missing `llvm-tools`
(`substrate/04-STORAGE.md`).*
