# KE16 task representation — the allocation-elimination register

The design lives in `taskrep_plan5.md`, a scratchpad file belonging to a different agent session
that is **not in this repository and will be collected.** This file exists so that when it is gone
the campaign still has a record: what shipped, what it rests on, what would falsify it, what is not
done — and, at least as important, **which of the plan's own numbers and anchors were wrong.** A
register whose value is that it records what was wrong must not launder its own corrections, so
every correction below names what the plan had said.

**STATUS AT THE TIME OF WRITING (2026-09-07).**

| Stage | State | Where |
|---|---|---|
| 1 — `Task { payload, execute }` queue element | **SHIPPED**, `897c812f` | §Ladder |
| 3a — the per-scope bump block, wired to nothing | **SHIPPED**, `d3dfcb5a` | §Ladder |
| 3y — the protector gate's window key | **SHIPPED**, `45dd0dbd` | §Ladder |
| 3b — the block wired to the scoped spawn path | **IN THE WORKING TREE, UNCOMMITTED** (7 `git status` entries) | §Ladder, §Sound |
| 3c — the Tree-Borrows negative controls (M2w) | **NOT WRITTEN.** Zero occurrences in the worktree | §Ladder, §Owed 2 |
| 3z — the release-window closer | **DEFERRED** behind a written trigger that has not fired | §Ladder |
| Exit conditions | **NONE RUN for this register.** 5 runnable today (1, 2, 3, 7, 8); 1 discharge-ready only in an owner-scheduled quiet window (10); 4 not reachable as written (4, 5, 6, 9) | §Exit |
| Performance number | **NONE TAKEN, AND NONE IS OWED BY THE SHIP RULE** | §NoTiming |

⚠ **THIS REGISTER IS A READING OF THE TREE, NOT A RUN OF IT.** No `cargo`, `clippy`, `miri` or
build command was executed to write it — another agent held the cargo lock and this is the owner's
workstation. Every "green" named below is **owed, not observed**, and every command is given so the
next reader can take the receipt rather than inherit it. The facts are from five read-only
verification agents that checked plan5's claims against `D:/wt/threadpool` file by file.

---

## §0. Environment and code state

### Tree

| | |
|---|---|
| Worktree | `D:/wt/threadpool`, branch `feat/threadpool-ke16` |
| HEAD | `c20f6883b8ae9498c201516fe369aaadce5e493e` — *"build(toolchain): the tree pins its own toolchain, and the docs job climbs above the pin"* |
| Working tree | **7 stage-3b entries** — `M src/block.rs`, `M src/lib.rs`, `M src/scope.rs`, `D src/task.rs`, `M src/thread_pool.rs`, `?? src/task/`, `?? tests/block_allocation_receipts.rs` (all under `crates/boyko_threadpool/`) — **plus this register itself**, `?? docs/threadpool/KE16-TASKREP-ALLOCATION.md`, so the command now returns **eight** lines |
| Plan base | `d647d930` — plan5's line citations were taken against this commit, which is why so many of them have drifted (§Corrections) |

`crates/boyko_threadpool/src/task.rs` **no longer exists.** 3b split it into `src/task/mod.rs`,
`src/task/scoped.rs` and `src/task/detached.rs`, which invalidates every `task.rs:NNN` anchor in
plan5 — **nine of them, across four plan5 sections, are corrected below** (§Corrections rows
6, 13 and 20). The only `thread_pool.rs` change is a comment path fix at
`crates/boyko_threadpool/src/thread_pool.rs:620` (`src/task.rs` → `src/task/mod.rs`).

### Toolchain, for every command in this file

⚠ **`cargo +nightly` resolves to `nightly-x86_64-pc-windows-MSVC` on this box and dies in the
linker with exit 1, which is indistinguishable from "the gate is red"** (`KE16-RESULTS.md` §0).
Every Miri command below therefore spells `+nightly-x86_64-pc-windows-gnu`.

⚠ **`RUSTFLAGS` must never be set** — `.cargo/config.toml` in this worktree carries the ISA
baseline on `[target.x86_64-pc-windows-gnu] rustflags`, and setting the environment variable
*replaces* it. Where `--cfg loom` is needed it goes through `cargo --config
target.x86_64-pc-windows-gnu.rustflags=[…]`, which JOINS. The same asymmetry applies to
`MIRIFLAGS`: `.cargo/config.toml:14-15` sets `MIRIFLAGS = "-Zmiri-tree-borrows"`, and an
environment `MIRIFLAGS` replaces rather than extends it.

---

## §What. The change, and what it costs

**Today**, one heap allocation per spawned task. `alloc_cell` is
`crates/boyko_threadpool/src/task/detached.rs:74-90` — the cell class is exactly `Layout::new::<C>()`
(`:76`) and the failure mode is `handle_alloc_error` (`:86-88`). *(Plan5's §3 class table and its
§2.3 Q4 answer cite `task.rs:334-348` and `task.rs:345-347`; both are **STALE** — the file is gone,
though both statements are still TRUE about the code. `alloc_cell` is now `pub(super)` with a single
remaining caller, on the detached path, at `src/task/mod.rs:292`.)*

**After 3b**, one allocation per chunk plus one free per chunk at the join. `grow` calls
`alloc(layout)` at `src/block.rs:445`; `free_all` (`src/block.rs:262-276`, loop at `:292-306`) is
called once, from `Scope::drop` at `src/scope.rs:1449`.

**The peak payment moves from tasks IN FLIGHT to tasks SPAWNED, and that is the cost of the
change.** Nothing is reclaimed before the join: `bump` refuses rather than splits
(`src/block.rs:348-350`) and `grow` overwrites the cursor pair with the new chunk
(`src/block.rs:458-462`), abandoning whatever is left of the chunk it leaves — the module says so
itself at `src/block.rs:1072-1075`. A scope that spawns N tasks therefore holds N cells' worth of
chunk bytes at its join even if only W of them ever ran at once. The abandoned tail is still
*freed* — `bases[i]` was recorded before the switch (`:449`) so `free_all`'s bound covers it — so
the model loses bytes, not chunks.

### Geometry

| | |
|---|---|
| `CHUNK0` | 4096 (`src/block.rs:83`) |
| `CHUNK_ALIGN` | 64 |
| `MAX_CHUNKS` | 32 (`src/block.rs:94`) |
| `required(T)` | `size_of + align.saturating_sub(CHUNK_ALIGN)` (`src/block.rs:228`) |
| `min_e` | `if required <= CHUNK0 { 0 } else { (required-1).ilog2() + 1 - CHUNK0.ilog2() }` (`src/block.rs:397-401`) |
| `cap_i` | `CHUNK0 << max(i, min_e)` (`src/block.rs:418-419`) |
| Table reach | total `4096 * (2^32 - 1)` = `2^44 - 2^12` = 17,592,186,040,320 B = **16 TiB − 4 KiB**; the last chunk alone is `2^43` = **8 TiB exactly** (`src/block.rs:89-93`) |

Because `e = min_e.max(i)`, a large `T` can make chunk `i` bigger than `CHUNK0 << i`, so 16 TiB is
the **doubling-path** capacity and a LOWER bound on the table's reach — which only strengthens
"exhaustion is unreachable while allocation succeeds".

`const _: () = assert!(usize::BITS >= 64)` (`src/block.rs:100`) is a real assertion only at this
table length: `4096 << 31` = `2^43` must not overflow, while at `MAX_CHUNKS = 16`, `4096 << 15` =
2^27 still fits in 32 bits and the same line would have had no arithmetic behind it. The comment at
`src/block.rs:89-100` states that reason, and `src/block.rs:402-418` adds a bound the plan does not:
`usize::BITS >= 64` bounds only the `i` term, `min_e` is bounded by rustc's own refusal to lay out a
type of size ≥ 2^61, and **`CHUNK0 << e` silently yields 0 (not a panic) for `e` in 52..=63 — which
is the real hazard the assert does not cover.**

⚠ **CORRECTION carried forward.** plan4 said `MAX_CHUNKS = 16` gives "268 MiB, three orders of
magnitude of headroom". It gives **256 MiB** (`4096 * (2^16 - 1)` = 268,431,360 B = 255.996 MiB —
"268 MiB" was the decimal byte count wearing a binary unit) and **2.3× headroom in chunks**
(16 available vs 7 used at the largest shape in the tree), not three orders of magnitude.

### The amended register sentences from plan5 §6, in the form the verification supports

**1. The per-scope bound.**

> Bounded by the scope: **384 KiB of payload, 508 KiB requested from the allocator**, at the largest
> shape in the tree (`crates/boyko_threadpool/tests/stress.rs:139`) — and bounded above only by the
> allocator, because `BatchingStrategy`'s public fields
> (`crates/boyko_ecs/src/ecs/core/iters/query/par_iter.rs:83-98`) make the task count a caller's
> choice.

Derivation, re-done independently by two verification arms and agreeing: the closure at
`tests/stress.rs:101-113` captures `DropTracker` (16 B, `:34-37`), `&AtomicBool` (8 B) and `id`
(8 B, live in the `assert!` message) = **F = 32 B**; the cell is `SCOPED_CELL_HEADER` 16 B
(`src/lib.rs:553`) + 32 = **48 B**; `run_exactly_once(8, 8192)` at `:139` gives 8192 × 48 =
393,216 B = **384 KiB exactly**, and the bump places 85/170/341/682/1365/2730/5461 cells per chunk,
cumulative 5,373 < 8192 ≤ 10,834, so **7 chunks**.

⚠ **THE TWO NUMBERS ARE BOTH IN THAT SENTENCE BECAUSE THE VERIFICATION CLUSTERS DISAGREED ABOUT
IT, and the disagreement is recorded rather than adjudicated.** Both computed the same arithmetic.
One cluster CONFIRMED plan5's sentence verbatim ("384 KiB at the largest shape"). The other REFUTED
it: 384 KiB is the *payload*, whereas the block requests each chunk whole
(`Layout::from_size_align(cap, CHUNK_ALIGN)`, `alloc(layout)`, `src/block.rs:437-445`), so the
requested total at that shape is `4096 * (2^7 - 1)` = 520,192 B = **508 KiB**, of which ~384 KiB is
ever touched (chunk 6 holds only 8192 − 5373 = 2819 of its 5461 cells). Its objection is specific:
the very same §6 paragraph says "on Windows the commit charge tracks the allocated figure", so a
bare 384 KiB is right for the resident column and **1.32× low for the column that paragraph itself
names as the one that grows.** Hence both numbers, always, with the words "payload" and "requested".

Residual on "largest in the tree": no site beating it was found. Production paths cap at ≤1024
cells (schedule), ~(W+1) per archetype (`par_iter`) and `n_waves` (physics:
`crates/boyko_physics/src/resources.rs:1781`, `:1873`; `src/soft/colored.rs:1040`;
`src/solver/colored.rs:2743`); the next largest in the crate is 4096 at `tests/stress.rs:133` and
`tests/diag_lane.rs:84`. **But an ECS-side `par_iter` over ~263+ archetypes of ≥16k rows would
exceed it**, which is the second half of the sentence being literal rather than rhetorical:
`BatchingStrategy`'s three fields are `pub` (`par_iter.rs:89`, `:94`, `:97`) and publicly
re-exported (`.../iters/query/mod.rs:74`); `{1,1,1}` makes `chunk_size` 1 (`:119-125`) so
`n_chunks == entity_count`, and `min_batch_size = 1` also defeats the inline cut-off at `:376`. At
4 M rows and stride 88 the block issues **17 allocations** (cumulative through i=15 is 3,050,349 <
4,194,304; through i=16 is 6,100,751) with a 17th chunk of `4096 << 16` = **256 MiB**, where today's
path issues **4 M allocations plus 4 M frees** over 352 MiB of payload. Robust to the reading of
"4 M": 4,000,000 also gives 17.

An adjacent fact the same public-fields argument implies and plan5 does not name:
`raw.clamp(min, max)` (`par_iter.rs:119-125`) **panics when `min > max`**, so the default
`min_batch_size = 1024` with a caller's `max_batch_size = 1` is a panic in library code on
public-API input, not merely a slow dispatch.

**2. The growth-waste sentence — REFUTED as the plan wrote it.**

plan5 §6 said "the doubling growth costs **up to 2×** the needed bytes in address space". 2× is only
the asymptote. Measured over the landed rule (`src/block.rs:418`, `:397-401`, `:83`):

| shape | allocated / needed |
|---|---|
| one 48 B cell | 4096 / 48 = **85.3×** |
| one 8 B cell | 4096 / 8 = **512×** |
| 48 B + one 4096 B value | 12,288 / 4,144 = **2.97×** |
| three chunks | 2.32× |
| four chunks | 2.14× |
| the plan's own 400-archetype `par_iter` shape | 1.75× |

> The doubling growth costs **at most about 3× the needed bytes in address space in the multi-chunk
> regime, decaying to 2× as the chunk count grows, plus a hard 4 KiB first-chunk floor per scope.**

This is *not* the oversized-cell case one would suspect: for `required > CHUNK0`, `min_e` is minimal,
so the fitted chunk is strictly < 2× `required` (`src/block.rs:397-401`), and the `max(i, …)` term
only inflates a chunk when the already-allocated bytes exceed it. **The pathology is small-n —
exactly the per-scope case the empty and one-task paths live in.**

The residency half of the same sentence — "of which only the touched half becomes resident; on
Windows the commit charge tracks the allocated figure" — is **NOT VERIFIED** and is not promoted
here. The readable half checks out: `grow` allocates and records only (`src/block.rs:445-462`, no
`write_bytes`, no memset) and `bump` only advances the cursor (`:332-361`), so the engine itself
never touches the untouched tail. The OS halves are not in this tree. **Settled by** measuring
`GetProcessMemoryInfo` (`WorkingSetSize` vs `PrivateUsage`), or perfmon Working Set / Private Bytes
/ Committed Bytes, around a probe that forces a ≥256 MiB chunk; whether the default allocator routes
that size to `VirtualAlloc(MEM_COMMIT)` is an allocator fact, not a repository fact.

**3. The M2o sentence — the observation gate over the completion protector.**

> The gate reads **3 of 4 scopes overlapping in a default run** on the shipped tree. The count is
> **printed rather than asserted**, so a drift from 3/4 toward 1/4 is visible before it reaches 0/4
> and fails; the shipped assertion threshold is `overlapped >= 1`.

⚠ **Cite it by function name — `assert_probe_armed` in
`crates/boyko_threadpool/tests/miri_scope_completion_protector.rs` — never by line.** plan5 cited
`:284-289`, which was CORRECT at its base commit `d647d930` and shifted **+12 lines** when 3y
(`45dd0dbd`) added `MIRI_WINDOW_SLOT_EXHAUSTIONS` to that file (+71/−6); the sentence is now at
`:296-301`. It has already rotted once inside a file whose *content* did not change, and citing by
name is the pattern plan5 §4 itself prescribes for the tb-neg receipts. Two further corrections in
the same neighbourhood: the range plan5 calls the "receiver × seed table" is `:88-93` and is a
receiver × **probe** × **rate** table (the seeds `{0,1,7,15}` are named separately at `:86`, which
plan5 cites correctly), and **five** of its six cells read 4/4 while the sixth reads "1/4 UB
(seed 7)" at the default preemption rate — so "reads 4/4" is nearly, but not entirely, right.

---

## §NoTiming. There is no performance number, and none is owed

**No nanosecond figure and no percentage exists for this change.** plan5 says so in as many words,
and the pre-committed rule is that the change **ships on R1 — the allocation-count receipt — not on
a timing.** If a later reader wants a speed number here, the correct text is that **none was
taken.**

Two adjacent figures that a reader will otherwise mistake for a stage-3b measurement — and a
third, §Ladder's stage-1 CS-2 ledger, which is transcribed from `KE16-RESULTS.md:179-180` and
belongs to stage 1:

* **The empty-scope cost is NOT RE-READABLE ON THIS TREE.** `ScopeBlock::new()` is a `const fn` that
  zero-initialises the 288-byte table (`bases` 32×8 + `exps` 32×1, `src/block.rs:141-149`) on every
  scope open (`src/scope.rs:1028`, `src/block.rs:163-172`), and zero-spawn scopes really exist:
  `par_iter.rs:341` and `par_chunk.rs:139` open the scope *before* the archetype loop with no
  `ids.is_empty()` guard, and three paths then spawn nothing — empty `ids`, `entity_count == 0`
  (`par_iter.rs:367-369`, `par_chunk.rs:171-173`), and every archetype under `min_batch_size`
  (inline at `par_iter.rs:376-391`). The default `min_batch_size = 1024` (`par_iter.rs:73`, `:105`)
  is overridden by **no** production code, so every sub-1024-row `par_iter` is a zero-spawn scope:
  `crates/boyko_demo/src/sim/systems/boids.rs:110`, `boids.rs:182`, `common.rs:54`,
  `particles.rs:69`, `physics.rs:100`, `physics.rs:374`. The **+13-instruction / ten-`vmovups`**
  figure that circulates for this path **cannot be re-read here — `probe_empty_scope` does not exist
  in this tree** — so it is recorded as *unverified on this code state*, and it is an instruction
  count rather than a timing in any case.
* Three qualifiers that travel with it: the schedule pays this **once per frame**, not per dispatch
  round (§Bound); the 288 B of zeroing is not required by the free contract, since `free_all` reads
  only `bases[..n]`/`exps[..n]` (`src/block.rs:292-293`) while `grow` writes both entries before
  publishing `n_chunks` (`:449-457`), so the cost is structural to `const fn new()`; and
  `src/block.rs:159-162` claims only that the empty path "makes no allocator call at all", which is
  true, and does not claim the zeroing is free.

The one thing in this campaign that *is* a timing — exit condition 10, the physics falsifier — is
deferred to an owner-scheduled quiet window and **has not been run** (§Exit 10).

---

## §Layout. The pinned numbers, and the pin that is missing

| Type | Value | Pin |
|---|---|---|
| `ScopeBlock` | **312 B**, align 8 — 8 + 8 + 4 + 4 + 256 + 32, no tail padding | `const _: () = assert!(size_of::<ScopeBlock>() == 312)` at `src/block.rs:154`; field list `:124-149` |
| `Scope<'scope>` after 3b | **328 B of content, 384 B at `align(64)`** — 8 (`inner`) + 8 (`shared`) + 312 (`block`) + 0 | size pin `src/scope.rs:1012`, **align pin `:1013`** |
| `Scope` before 3b | **16 B** (`&PoolInner` 8 + `NonNull` 8 + `PhantomData` 0) | none; default repr |
| `Task` | **16 / 8 / 16** (`size_of`, `align_of`, `size_of::<Option<Task>>`) | `src/task/mod.rs:199-203`; the niche comes from `TaskFn = unsafe fn(*const ())` at `:165` |

Independently re-derived offsets under `repr(C)` (`Cell<T>` is `repr(transparent)`):
`inner`@0, `shared`@8, `cur`@16, `end`@24, `n_chunks`@32, `_pad`@36, `bases[0]`@40, `exps`@296.

Three corrections and one gap:

1. ⚠ **The align pin is the tree's, not the plan's.** plan5 specified only the size pin;
   `src/scope.rs:1008-1011` states the right reason for the second one — a size pin alone survives
   the alignment being dropped, which is the edit that silently turns the *guaranteed* cache line
   back into a probability. The tree in fact carries **three** pins plan5 never specified (this one
   plus the two cell ALIGN literals, §Receipts).
2. ⚠ **The hot span is bytes 0..32, not 0..40 — plan5 §2.1 is REFUTED.** A non-growing spawn
   touches `inner`, `shared`, `cur`, `end` only: `bump` (`src/block.rs:332-361`) never mentions
   `n_chunks`, which is read exclusively at `:292` (`free_all`), `:390` (`grow`) and `:188`
   (`is_empty`). Harmless to the conclusion — 0..40 and 0..32 both sit inside one 64-byte line — but
   the plan overstates the hot working set by two fields, and **the tree has already corrected
   itself in prose** (`src/block.rs:113-119`, `src/scope.rs:963-968`), leaving plan5 as the only
   place that still says 0..40.
3. ⚠ **The pre-3b `Scope` anchor is STALE and it is the anchor plan5 uses twice**, in §2.1 and in
   its own §6 correction table — the two places a reader goes to re-verify. The value 16 B is right;
   `scope.rs:748-766` is not. At HEAD the struct is `src/scope.rs:901-919` (span `:893-919`); in the
   working tree it is `:957-1003`. Line 748 was correct only up to `d3dfcb5a`; 3y (`45dd0dbd`)
   inserted ~153 lines above it, and lines 748-766 at HEAD are now the middle of a `// SAFETY:`
   block inside `ScopeShared::complete_task`.
4. ⚠⚠ **NOTHING PINS THE FIELD ORDER, and the field order is the property that was bought.** plan5
   §2.1 says "Drift is a build failure" of the size pins; `src/scope.rs:1005-1013` repeats it for
   size and alignment. **REFUTED:** declaring `block` first — `block`@0, `inner`@312, `shared`@320,
   `_phantom`@328 — still gives 328 content bytes, `size_of == 384` and `align_of == 64`, so **both
   pins stay green** while `inner`/`shared` move to bytes 312..328 and the two hot pairs straddle two
   cache lines. A crate-wide search for `offset_of` returns only `src/task/mod.rs:573-594` (the cell
   head offsets): there is no offset pin over `Scope` or `ScopeBlock`. **`offset_of!(Scope, block)
   == 16` would close it at zero cost** (§Owed 8).

The one-cache-line claim itself is CONFIRMED given the align pin — all four hot fields lie in bytes
0..64 — with one qualification the prose does not carry: `bases[0..=2]` (@40, @48, @56) are **also**
on that line, so the "288-byte table behind them" is not spatially disjoint from the hot line for
its first three entries. That costs nothing (they are read only by `free_all`), but the line is not
*exclusively* hot.

`Task`'s three pins are correct and are the tree's own: plan5 does not state them anywhere.

---

## §Bound. The per-scope bound and its call sites

| Site | Scope granularity | Tasks | Block bytes | Chunks |
|---|---|---|---|---|
| `crates/boyko_ecs/src/ecs/core/schedule/schedule.rs:455` (`pool.install`) | ⚠ **ONE scope per `Schedule::run`, i.e. per FRAME** — *not* "per dispatch round" | ≤ 1024 | 1024 × 56 B = 57,344 B = **56.0 KiB** | 4 |
| `.../iters/query/par_iter.rs:341`, `.../par_chunk.rs:139` | one scope over ALL archetypes | ~(W+1) per archetype | 1.5 KiB × archetypes | 6 @ 100 arch, 8 @ 400 arch (at an 88 B cell); **5 and 7 at a 72 B cell** |
| `crates/boyko_fontbake/src/msdf/distance.rs:433` (`pool.install`) | one scope per glyph | `min(4W, h)` bands = 64 at W=16 | — | — |
| `crates/boyko_threadpool/tests/stress.rs:139` | one scope | 8192 | **384 KiB payload / 508 KiB requested** | 7 |

⚠ **"Per dispatch round" is REFUTED.** The rounds are *inside* the closure: `schedule.rs:641` is
`loop {` and `:722` passes the **same** `&Scope<'scope>` every iteration (signature `:1036-1041`),
with the frame's only other exit at `:712-714`. The block accumulates across every dispatch round of
the frame. **The anchor `:455` is exactly right — it is the `pool.install` line — so this is a wrong
label on a correct citation**, and it is the label that would make a re-derived bound too small by
the number of rounds in a frame.

The stated ceiling survives anyway, and for a reason worth writing down: **its author used the
frame-wide system count, not per-round concurrency.** Each system index contributes at most one cell
per frame (`schedule.rs:1062-1067` skips anything already completed or running; dispatcher-only
systems go to `exclusive_to_run` and run **inline** at `:1131-1143`, spawning nothing). So no reader
who takes the NUMBER is misled — only one who re-derives from the LABEL.

Two corrections and one hole in that row:

* The unit. 1024 × 56 B = 57,344 B = **56.0 KiB**; plan5's "57 KiB" is the decimal figure (57.3 kB)
  wearing a binary unit. At a 48 B cell it is 49,152 B, also 4 chunks.
* **The ≤1024 cap is a `debug_assert!` only.** The constant is
  `.../schedule/schedule_builder.rs:71` (`MAX_SYSTEMS_PER_SCHEDULE = 1024`), the citation
  `schedule.rs:1007` is exact, but enforcement is `debug_assert!` at `schedule_builder.rs:491-496`
  plus the `u16` `SystemIndex` truncation at `schedule.rs:1102`/`:1110`. **56 KiB is therefore not a
  release-enforced ceiling** — consistent with §2.3's own "the bound is the API, made unreachable
  rather than audited", and a second reason the row must not be read as a bound.
* **The low end, "3.6 KiB", is UNVERIFIABLE and is not promoted to a fact.** It is not re-derivable
  from the row's own inputs: 1..=1024 tasks at 48–56 B spans 48 B .. 57,344 B, and 3.6 KiB
  corresponds to ~66–76 cells, a count that appears nowhere in the row. A plausible but unstated
  derivation is ~76 concurrent systems × 48 B = 3,648 B — the engine's *real* schedule width, not a
  formula endpoint. **Settled by** instrumenting `Schedule::run` to print `self.systems.len()`, or
  by tallying the 94 `add_systems` registration sites (`grep -rn "add_systems" crates/*/src`) — not
  by reading.

Nested scopes do **not** add to an enclosing block: `PoolInner::scope` (`src/thread_pool.rs:378`)
and `PoolInner::install` (`:333`) both do `Scope::new`, which sets `block: ScopeBlock::new()`
(`src/scope.rs:1017-1031`) inline in the `Scope` (`:998`), and a spawn emplaces into *this* scope's
block (`:1327`). A nested fan-out is charged to its own block and freed by its own `free_all`
(`:1449`), so the schedule scope's 56 KiB and a `par_iter`'s block never sum into one allocation.

Two premises the `par_iter` row does not state: (W+1) per archetype holds only at
`entity_count >= min_batch_size * W` (16,384 rows at W=16); an archetype in 1024..16384 rows gives
`ceil(n/1024) < W+1`, and one below `min_batch_size` spawns nothing at all.

---

## §Sound. The soundness position, and the clause that was lost

**Before 3b the scoped cell was freed as soon as the body had been read out of it, before the body
was invoked — so by the time the release RMW committed, the payload allocation did not exist at
all.** That is strictly stronger than "no protector covers it", because "the allocation does not
exist" needs nothing from the code that runs afterwards. **That clause is GONE for the scoped kind.**

The tree states it AS a loss rather than rewording it into a win, which is the one paragraph a
future reader will use to decide whether the argument got weaker:
`crates/boyko_threadpool/src/task/mod.rs:117-140` — heading *"THE CLAUSE THE SCOPED PATH LOST, named
as a downgrade"*, the dead claim block-quoted, `:126` recording that "the replacement is weaker in
FORM, which is why it is written as a loss rather than reworded into a win", and the replacement
clause quoted at `:133-135`. *(plan5's anchor `task.rs:78-85` is STALE; the file is gone.)*

**What replaces it: D1 (placement), D2 and D3 (type-system facts), D5 (a reduction to exactly one
function) and M2w (an execution gate over that function) — and M2w does not exist yet, which is the
campaign's largest open risk.**

### The four that landed

* **D1 — chunks hold payload bytes only.** No `next`/`used`/`cap`/guard word; capacities live in
  `ScopeBlock::exps` in the joiner's frame. Field list `src/block.rs:124-149`; the single chunk write
  is `ptr::write(typed, value)` at `:249`, *before* `BlockPtr` exists and therefore before `erase`
  and before the push (the ZST arm at `:222` writes to `NonNull::dangling()`, not a chunk); `bump`
  writes only `cur` (`:358`); `grow` writes only inside the 312-byte struct (`:449-462`); `free_all`
  writes only `n_chunks`/`cur`/`end` (`:310-312`). `mod block;` is private (`src/lib.rs:63`) and all
  six fields are private, so no other module can write one. The three-site wiring claim holds
  exactly: `src/scope.rs:1327`, `:1425` (`cfg(miri)` `is_empty`), `:1449`, plus `ScopeBlock::new()`
  at `:1028`.
* **D2/D3 as type-system facts.** `emplace` allocates AND initialises
  (`src/block.rs:207-253` — value in at `:249`, handle constructed at `:252`), so no caller ever
  holds a pointer to uninitialised block memory; the **only** `impl` for `BlockPtr` is
  `src/block.rs:515-525`, one method, `erase(self) -> *const ()` by value, with no `Deref`, no
  `From`, no `as_mut` and no getter anywhere in the crate; `ScopedCell` is `pub(super)` inside
  `crate::task::scoped` (`src/task/scoped.rs:21-26`), i.e. `pub(in crate::task)` at most, pinned as
  a consequence in a test comment at `src/task/mod.rs:566-571`.
* **The replacement clause itself**, verified statement by statement over
  `src/task/scoped.rs:72-166`: `:73` inherent raw-pointer methods (`self` by value, no autoref);
  `:83` a `Copy` place read of a pointer field; `:84` `ptr::read(ptr::addr_of!((*cell).body))`;
  `:90` `catch_unwind(AssertUnwindSafe(body))` over a body **already moved out of the cell**, so the
  protected aggregate is in this frame; `:92` a by-value match on a frame local; `:165`
  `ScopeShared::complete_task(shared)`, an associated fn over `*const Self` (`src/scope.rs:770`).
  None of §4's invisible protector mints is present — no `ref`/`ref mut` pattern, no `&mut *p`, no
  `format_args!`, no `assert_eq!`/`debug_assert_eq!`, no operator on a non-primitive, no closure
  capturing a place in the cell. One autoref *does* exist and is harmless: `:98`
  `(*shared).capture_panic(payload)` autorefs `&ScopeShared` (receiver `src/scope.rs:571`) — over the
  `ScopeShared` allocation, not the cell, on the `Err` arm only, and its activation returns before
  `:165` while this task's own registration still counts in `pending`, exactly as the SAFETY comment
  at `:93-97` says.
* **The five-step activation list lost step 2 by DELETION, not by softening.** At `d647d930` it read
  "2. The cell is freed here, before the body runs."; the landed list
  (`src/task/scoped.rs:52-59`) has four steps, no free step, and the one internal cross-reference
  decremented in step ("that activation RETURNS at **step 4's** semicolon" at `d647d930` →
"returns at **step 3's** semicolon" as landed), with
  the deletion stated in prose at `:61-65`. **Residual, carried rather than introduced:** the
  cross-reference points one step PAST the statement it describes in *both* revisions — the
  `catch_unwind` activation ends at its own statement's semicolon (step 2 in the new numbering), not
  at step 3's. Harmless to the argument, and a sentence a careful reader will trip on.
* **D5 is extended into `run_scoped`'s existing two-activation-stack SAFETY block, not duplicated**
  — `src/task/scoped.rs:133-164`, inside the same comment whose WORKER/JOINER stacks are at
  `:110-127`, with the four enumerated frames at `:140-158` and the conclusion at `:160-164`. A
  crate-wide search finds the enumeration only there. It reduces the property to exactly one
  function, and **no frame was found that defeats the reduction**: nested scopes free only their own
  chunks; crossbeam moves `Task` bytewise and never mints a reference into a chunk; the joiner's
  `&PoolInner` / `&Worker<Task>` / `&mut Scope` are all outside chunk memory.

### ⚠ Two places where the written argument is wrong or short

* **D2's UNQUALIFIED sentence is REFUTED.** `src/block.rs:44-51` says "no caller can form a
  REFERENCE into chunk memory … checked by the type system, not by a grep". That is true of every
  route **through `BlockPtr`** and false as a statement about the crate: `drop_unrun_scoped`
  (`src/task/scoped.rs:182-192`) does `ptr::drop_in_place(ptr::addr_of_mut!((*cell).body))` at
  `:191`, and the drop glue it invokes calls the user's `Drop::drop(&mut self)` — **a `&mut F` whose
  referent IS chunk memory, passed as an argument, i.e. a genuine strong protector over a chunk for
  the whole of the user's destructor.** Its safety rests on **ordering** (an unrun task's
  registration is still counted, so `free_all` cannot run), never on "no reference is formed" — and
  the tree carries that correct argument three lines away, at `src/task/scoped.rs:170-175` ("not
  merely forbidden but unreachable-by-order"). The two sentences are simply not reconciled, and it
  matters precisely because the register sentence would otherwise read "no reference into chunk
  memory exists". **The type-system half of D2 is intact.**
* **D5's enumeration is INCOMPLETE AS WRITTEN, not wrong.** Item 1 says "the only frames that
  receive it are `Task::run` → `execute(payload)`"; `Task::drop` (`src/task/mod.rs:359-371`) also
  receives the chunk address — as a field of a `&mut Task` argument — and calls
  `(head.drop_unrun)(self.payload)`. Harmless, because `&mut Task`'s protector covers the 16-byte
  element and `payload` is copied out as a raw pointer without retagging the pointee, and vacuous by
  item 2's own ordering argument. The second omission is the drop-glue `&mut F` above, which item 2
  covers by ordering but which the enumeration does not name as a reference at all.

### `Scope::drop` — the order, and the panic-free interval

The order in `src/scope.rs:1383-1529` is exactly §2.5's, with every anchor moved:

| step | plan5 | landed |
|---|---|---|
| join | `:1135` | **`:1397`** |
| key read | — | **`:1413`** — `(*raw).miri_key`, **not `raw as usize`**: this is the 3y correction |
| `is_empty` / block note | — | **`:1425`** / **`:1426`** |
| `free_all` | — | **`:1449`** |
| `debug_assert!(is_drained())` | `:1146-1149` | **`:1460-1463`** |
| `take_panic()` | `:1151` | **`:1465`** |
| M1 note | `:1200` | **`:1520`** |
| `Box::from_raw` | `:1202` | **`:1522`** |

**NOTHING BETWEEN THE JOIN AND THE FREE CAN PANIC — CONFIRMED.** In a non-miri build there are
**zero** statements between the join's return at `:1397` and `free_all` at `:1449`. Under `cfg(miri)`
there are three, all panic-free: `:1413` a `Copy` place read; `:1425` `is_empty`, whose body is
`self.n_chunks.get() == 0` (`src/block.rs:187-189`); `:1426` the block note, whose body is
`.iter().any(…)` over a fixed 8-element array plus one `fetch_add` (`src/scope.rs:423-432`) — no
indexing, no allocation, no unwrap. The reason is stated at `:1429-1431`. `free_all` is
panic-audited in place at `src/block.rs:262-276`, which correctly identifies the range slice as the
only candidate and shows it cannot fire.

⚠ **Residual: the leak-safety claim survives on the abort backstop, not on structure alone, and
that dependency is not written next to it.** `ScopeBlock` has no `Drop` (`src/block.rs:69-74`), so a
panic raised **inside** the join would skip `free_all` and leak every chunk. No such path was
reachable: a task body's panic is captured by `run_scoped` (`src/task/scoped.rs:90-99`), and anything
escaping `Task::run` is caught and aborted by `worker::run_task` → `abort_on_task_panic`
(`src/worker.rs:183-218`), which is also the path `drain_scratch` takes (`src/scope.rs:2064-2071`).

### The release window — every anchor STALE, and one mechanism changed

plan5 §2.5 says the release window does not open at `pending.fetch_sub` but at the successful CAS
inside `miri_release_probe`, and that the interval `(t_dec, t_open)` contains no yielding or blocking
operation. **The claim's substance holds; every citation and one mechanism do not:**

| | plan5 | landed |
|---|---|---|
| releasing `fetch_sub` | `:599` / `:668` | **`:805`** (W-d′ arm) / **`:885`** (external arm) |
| window OPENS | `:246` | **`:346-349`** — `compare_exchange(MIRI_WINDOW_SLOT_EMPTY, key, SeqCst, SeqCst)` |
| firing counter | `:240` | **`:339`** |
| `unpark` | `:620` | **`:826`** (W-d′); the external arm's is at **`:868`, BEFORE the decrement**, so that arm's interval is shorter still |
| window CLOSES | "the slot store (`:257`)" | **`:368-375` — a `compare_exchange(key → EMPTY)`, NOT a store** |

The store→CAS change is 3y's, and deliberate: `src/scope.rs:216-223` records that the expected-value
CAS is what makes "ADDING a closer later — a `Scope::drop` that clears its own scope's slots after
the last reclamation — a two-line change rather than a correctness re-argument". First yield is the
burst at `:358-360`, after the CAS, as plan5 says.

**The code half of the precondition is confirmed by reading**: the interval on both arms contains
only a branch, one atomic `fetch_add`, and on one arm a `Thread::unpark` (`WakeHandle` =
`std::thread::Thread` under `cfg(not(loom))`, `src/sync.rs:113`) — no `yield_now`, no park, no lock,
no allocation. **The remaining half is NOT DECIDABLE FROM THIS TREE**: whether Miri at
`-Zmiri-preemption-rate=0` treats `unpark` as a preemption point. **Settled by**

```
MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-permissive-provenance -Zmiri-ignore-leaks -Zmiri-preemption-rate=0" \
  cargo +nightly-x86_64-pc-windows-gnu miri test -p boyko-threadpool \
  --test miri_scope_completion_protector -- --nocapture
```

and reading the printed counts. **NOT RUN.**

### ⚠⚠ M2w — the campaign's largest open risk

plan5 §2.5 says: *"Precondition monitor, not faith. … What reports it is the printed counter:
`block_overlaps` is printed alongside `overlaps=` on every run, by a sibling of
`assert_probe_armed`, whose shipped threshold is `overlapped >= 1`."*

**REFUTED. `block_overlaps` occurs ZERO times in the crate.** The counter half of M2w landed in 3b —
`MIRI_BLOCK_FREES_INSIDE_A_RELEASE_WINDOW` at `src/scope.rs:295-297`, incremented in
`miri_note_block_free_against_open_windows` at `:423-431` from its single call site at `:1424-1427`,
with a public reader at `src/lib.rs:362-363` — **but nothing prints or asserts it.**
`assert_probe_armed` (`tests/miri_scope_completion_protector.rs:315-384`) prints only
`firings=`/`overlaps=`/`slot_exhaustions=` (`:342-345`) and asserts `fired >= expected` (`:351`),
`exhausted == 0` (`:360`) and `overlapped >= 1` (`:370`). There is no `tb_neg_m2w_*` binary.

**So the counter is incremented and read by nothing, and §2.5's argument — which leans on the
monitor twice — is faith today.** This is 3c's boundary rather than a 3b defect, but it must be
written where the claim is made, because **the counter EXISTING is exactly what makes an unmonitored
counter easy to mistake for a monitored one.** What is owed is a ~10-line test-side reader, not
3c-scale work (§Owed 1).

---

## §Ladder. The stages and their true state

**Stage 1 — SHIPPED, `897c812f`** *"feat(threadpool,ecs,physics): the KE16 candidate arms, and a
task representation that cannot outlive its own release"*. The queue element became
`Task { payload: *const (), execute: unsafe fn(*const ()) }`, 16 B. That commit also carries the
KE16 candidate arms, so it is not a clean stage-1 diff. `KE16-RESULTS.md` §CS calls the post-stage-1
code state **CS-2** and records stage 1's effect — at instruction level, read off the disassembly and NOT off a
timing — as **"+3 instructions and +8 B per spawn, −1 level in the call's dependency
chain"**, which is not an instruction win.

⚠ **That figure is TRANSCRIBED from `docs/threadpool/KE16-RESULTS.md:179-180` (CS-2 is
defined at `:158`) and belongs to STAGE 1, not to 3b.** No verification arm of this register
re-took it, and it is the only quantified performance-shaped number in this file — which is
why it is labelled here rather than left to read as a 3b measurement (§NoTiming).

**Stage 3a — SHIPPED, `d3dfcb5a`** *"the per-scope bump block, wired to nothing — and the positive
control that keeps its emptiness assertions honest"*. The block exists; no spawn path reaches it.

**Stage 3y — SHIPPED, `45dd0dbd`** *"the protector gate's window key was an address the allocator
recycles — and the defect is latent, measured rather than assumed"*. **Why:** the release-window
slot was keyed by an *address*, so the allocator was free to recycle one across two scopes and the
gate would certify an overlap that never happened. The fix keys the window on `(*raw).miri_key`
(`src/scope.rs:1413`) and closes it with an expected-value `compare_exchange(key → EMPTY)`
(`:368-375`) instead of an unconditional store. **The defect it repaired was measured LATENT, not
live** — that reading is the commit's own subject line and is transcribed from the campaign record;
the verification confirmed the *mechanism* in the tree (the key, the CAS, and the rationale at
`:216-223`), not the latency measurement.

**Stage 3b — IN THE WORKING TREE, UNCOMMITTED.** The seven stage-3b `git status` entries in
§0 (this register is the eighth line and is not part of the stage). It wires the
scoped spawn path to the block (`src/scope.rs:1327`), splits `task.rs` into `src/task/{mod,scoped,
detached}.rs`, adds the layout receipt literals and the `block_allocation_receipts` binary, and
ships the §Sound downgrade.

**Stage 3c — NOT WRITTEN.** A whole-worktree search for `tb_neg|tb-neg-m2w|run_scoped_neg|finish_neg`
across `*.rs`, `*.toml`, `*.ps1`, `*.sh`, `*.md` returns **zero hits**; `scripts/` holds 11 entries
and none is `tb_neg_gate.ps1`; `docs/threadpool/receipts/` does not exist. Itemised, in one commit:

1. `run_scoped_neg::<F>` plus `#[inline(never)] fn finish_neg<F>(&ScopedCell<F>, *const ScopeShared)`
   in `src/task/scoped.rs`, beside `run_scoped` (`:72`).
2. The `execute` swap inside `Task::new_scoped` (`src/task/mod.rs:262-266` and its body).
3. The `tb-neg-m2w` feature row in `Cargo.toml [features]` (after `:38`).
4. The `#[cfg(all(feature = "tb-neg-m2w", not(miri)))] compile_error!` in `src/lib.rs`.
5. `tests/tb_neg_m2w_block_reference.rs`.
6. `scripts/tb_neg_gate.ps1` (and `.sh`).
7. `docs/threadpool/receipts/tb-neg-m2w-{0,1,7,15}.stderr`.
8. `tests/tb_neg_m2w_arm_present.rs`.

⚠ **plan5 pairs 3b and 3c in ONE commit precisely because 3b alone ships the §Sound downgrade with a
positive-only column — and 3b is in the working tree while 3c is not. That pairing is currently
violated in the tree, though not in history.**

**Stage 3z — DEFERRED, behind a trigger that has not fired.** It is absent:
`src/scope.rs:162-163` still holds `#[cfg(miri)] const MIRI_RELEASE_PROBE_YIELDS: usize = 16;`, the
burst at `:358-360` is still `for _ in 0..MIRI_RELEASE_PROBE_YIELDS { std::thread::yield_now(); }`,
and no `miri_close_windows` exists anywhere. **The trigger is exit condition 5 reading
`overlaps=0/2`, and there is no fallback if it does**, because re-tuning the yield count is refused
in writing at two sites — `src/scope.rs:139-146`:

> ARMED-NESS IS NOT MONOTONIC IN THIS NUMBER … reconstruction B is DISARMED at 8 … while armed at
> 4, 5, 6, 7, 10, 12, 16, 24 and 32 … NO constant threshold can certify it

(repeated at `src/task/detached.rs:62-69`). 3y did land the substrate that makes 3z cheap if the
trigger fires: `src/scope.rs:216-223`, quoted in §Sound.

⚠ **3z was DEFERRED, not reverted** — which is why exit condition 5's written fallback ("if 3z was
reverted, revert to plan4's form") does not apply and the condition must be run in plan4's form on
this tree (§Exit 5).

---

## §Receipts. R0, R0b, R1, R2, R4′ — currency, and what makes each RED

All five live in **one** `#[test]` in `crates/boyko_threadpool/tests/block_allocation_receipts.rs`
(`:549-550` — the only test in the binary). The instrument is a `#[global_allocator]` shim
(`:303-321`) that touches four atomics and forwards to `System`; `realloc` and `alloc_zeroed` are
**deliberately not overridden** so their default bodies stay counted (`:309-312`).

| Counter | Type | Site | Read by |
|---|---|---|---|
| `CELL_ALLOCS` | `AtomicUsize`, cumulative | `:267` | **R0** (arming) and **R1** (zero) |
| `CHUNK_ALLOCS` | `AtomicUsize`, cumulative | `:269` | R0b, R2, R4′ |
| `CHUNK_LIVE` | **`AtomicIsize`**, net | `:279` | R4′ |
| `OTHER` | `AtomicUsize` | `:283` | printed only (`:660-661`, `:908-911`), never asserted |
| `CELL_SIZE`, `CELL_ALIGN` | the cell-class discriminant | `:259-261` | the bucket predicate |

The **Read by** column is derived from what each receipt asserts, not from a verification
finding — no arm of the verification assigned counters to receipts.

⚠ **plan5 §3's "four `AtomicUsize` statics, not a map" is REFUTED: it is three `AtomicUsize` plus one
`AtomicIsize`, and six atomic statics in total** counting the two discriminants. The departure is
documented at the site (`:271-278`) and is the better engineering — "a net counter that went
negative would wrap to ~2^64 in a `usize` … Both spellings are RED in the same cases; this one
prints why". It is recorded here anyway because **it is an exact-equality gate's own instrument, and
the register should say which type it is.** The "not a map" rationale stands: a map inside
`GlobalAlloc` re-enters the allocator it measures.

Buckets: the cell bucket is the pair `(size, align) == (CELL_SIZE, CELL_ALIGN)`, set per phase; the
chunk bucket is `align == 64 && size.is_power_of_two() && size >= 4096` (`:288-292`).

| Receipt | Asserts | Currency | RED when |
|---|---|---|---|
| **R0** | the arming allocation **appears** in the cell bucket | count of allocator calls | the positive control does not fire — which is what stops R1's zero from being green from an empty bucket |
| **R0b** | the arming allocation appears in the **chunk** bucket | count | ditto for R2 |
| **R1** | **zero** allocations in the cell bucket, at **two entry points** — `spawn` (`:747-754`) and `spawn_batch` (`:766-771`) | count of allocator calls — **not bytes, not time** | any spawn still allocates per task |
| **R2** | observed chunk-bucket count **equals** the test's own re-derived prediction; `R2_ROWS = [(1,1),(16,1),(64,2),(1024,5)]` (`:437`), build-time pins `:441-451` | count | the growth rule and the model disagree |
| **R4′** | whole-process net balance `CHUNK_LIVE == 0`; the panic phase additionally asserts `CHUNK_ALLOCS == 1` (`:876-879`) so the balance cannot be green from emptiness | net count | a chunk leaks, or the arming chunk never happened |

**R1 IS THE BINDING RECEIPT. The change ships on it, not on a timing** (§NoTiming).

`reset_cumulative` (`:360-364`) touches `CELL_ALLOCS`/`CHUNK_ALLOCS`/`OTHER` and deliberately **not**
`CHUNK_LIVE`, which is what makes R4′ (`:907-919`) a whole-process balance.

### The model is stated three times, independently

`predicted_chunks` (`:401-425`) is a `const fn` written **in the test**, with its own
`while (CHUNK0 << min_e) < required` loop and `placed += cap / stride` — it cannot call `block.rs`
(`ScopeBlock` is `pub(crate)`), and the binary's only crate imports are `ThreadPoolBuilder` and the
`__layout_receipt` constants (`:103-107`), re-pinned to literals at `:119-125`. `block.rs` carries a
third statement, `model_chunk_caps` (`:1080-1097`), which spells the literals out rather than reading
`CHUNK0` for exactly this reason. The run compares all three against the allocator.

Stride 88 is committed from two closure types measured at 72/1 and 80/8 (`:150-158`, with runtime
`size_of_val`/`align_of_val` probes at `:557-596` asserting them **before** anything derives from
them). The worked table re-checks exactly: 4096/88 = 46, 8192/88 = 93, 16384/88 = 186, 32768/88 =
372, 65536/88 = 744; cumulative 46/139/325/697/1441; so n=1 → 1, n=16 → 1, n=64 → 2, n=1024 → 5. The
floor model is legitimate at that stride because 88 % 8 == 0 and chunk bases are 64-aligned, so
`pad` is 0 for every cell (`src/block.rs:344`).

### Bucket disjointness — and the part only the run can decide

The chunk bucket is hit by nothing else in the crate's dependency set: crossbeam-deque 0.8.7's
`Block<Task>` is 8 + 63×24 = 1520 B at **align 8** and every `Buffer<Task>` is 16×cap at **align 8**
(both take alignment from `align_of::<T>()`, and `align_of::<Task>() == 8` is pinned at
`src/task/mod.rs:200`); `CachePadded` is `repr(align(128))` on x86_64 (crossbeam-utils 0.8.22), and
`ScopeShared` contains one (`src/scope.rs:454`, `:462`) so it is align 128 / size 256 and fails
**both** halves of the predicate — `src/scope.rs:676-678` already records that.

⚠ **The disjointness rests ENTIRELY on `align == 64`.** `Buffer<Task>` starts at `MIN_CAP` 64 ×
16 B = 1024 B and doubles, so it reaches 4096, 8192, … — sizes that satisfy the
power-of-two-and-≥-4096 half. And the residual plan5 already concedes cannot be settled by reading
at all: that **no other allocation in the whole process** (libtest, std, the panic hook) lands at
`(2^k >= 4096, 64)`. **Only the run decides it, and R0b/R2's exact equalities are that decision.**

### R3 and plan4's `total_allocs` halves were DELETED

They asserted over crossbeam's injector — an allocator this design does not own — so they would have
been flaky rather than binding, and **a flaky gate is worse than a red one.** The reason is
transcribed from the campaign record; no verification finding covers the deletion. What the
verification *does* establish is consistent with it: the binary has exactly one `#[test]`, holding
five receipts, and no `total_allocs` counter among its six atomic statics.

### The seven layout literals

`__layout_receipt` (`src/lib.rs:550-586`) publishes **seven** literals — `SCOPED_CELL_HEADER` 16,
`SCOPED_CELL_ALIGN` 8, `DETACHED_CELL_HEADER` 8, `DETACHED_CELL_ALIGN` 8, `CHUNK0` 4096,
`CHUNK_ALIGN` 64, `MAX_CHUNKS` 32 — each pinned by a `const _: () = assert!(…)` next to the real
type: `src/task/scoped.rs:34`, `:36`, `:44`; `src/task/detached.rs:28-32`, `:35-36`;
`src/block.rs:107-109`. The external binary imports them rather than recomputing
(`tests/block_allocation_receipts.rs:104-121`). Re-derived independently: `ScopedCell<()>` = 16,
`ScopedCell<[u8;64]>` = 80, `DetachedCell<()>` = 8, `DetachedCell<[u8;64]>` = 72, align 8 for all
four.

**Two different pairings, both sound, not the same device.** The four cell literals are pinned
against the **type** (`size_of::<ScopedCell<()>>()` etc.). The three chunk literals are pinned
against `block.rs`'s own module constants — equally binding only because those constants are the ones
the code under test uses (`bases: [_; MAX_CHUNKS]` at `src/block.rs:141`, `CHUNK0 << e` at `:419`,
`Layout::from_size_align(cap, CHUNK_ALIGN)` at `:437`, `free_all` at `:294-306`).

⚠ **plan5 §3 lists FIVE literals, not seven, and never mentions the ALIGN pins**
(`taskrep_plan5.md:349-354`; `:340` leaves `CELL_ALIGN` "set by the test per phase"). Its double-pin
argument is stated only for `ScopedCell`. **The direction of the error matters:** leaving the
alignment half of the receipt bucket to the test binary is *precisely* the vacuity
`src/lib.rs:558-563` names — a cell class free to move its alignment leaves the receipt counting
over an **empty bucket**, and "zero allocations" comes back green from emptiness. The
implementation caught this and fixed it (three pins per cell header, `src/lib.rs:543-548`); **the
plan is the stale side, and its literal list must be amended to seven before anyone audits the
receipt against it.**

---

## §Exit. The ten conditions, and which are actually reachable

**NONE of these was run for this register.** Three states are separated below, because collapsing
them is how a campaign ships on a condition nobody could have satisfied.

| # | State | One line |
|---|---|---|
| 1 | **NOT YET RUN** (anchors corrected) | needs `--features ke16-c-batch --release`, and `--exact` |
| 2 | **NOT YET RUN**, discharge-ready | the five receipts, both feature arms |
| 3 | **NOT YET RUN** (anchors corrected) | Miri with leak checking ON, filtered |
| 4 | **CANNOT BE RUN AS WRITTEN** | no `block_overlaps` reader exists |
| 5 | **CANNOT BE SATISFIED AS WRITTEN**, and must be run in plan4's form | 3z was deferred, not reverted; **no fallback if it goes red** |
| 6 | **CANNOT BE RUN** | 3c does not exist |
| 7 | **NOT YET RUN** (one citation and one attribution corrected) | loom M1/M1c |
| 8 | **NOT YET RUN**, N/A but cheap | a compile check |
| 9 | **VACUOUSLY GREEN TODAY** | fails for the wrong reason |
| 10 | **CANNOT BE RUN ON THIS MACHINE NOW** | owner-scheduled timing on a verified-idle box |

**1. The two `k > n` deciders, under the arm that compiles them.** In the default build
`spawn_batch` is `for f in bodies { self.spawn(f) }` (`src/scope.rs:1097-1108`) and
`WaveRegistration`/`charge_one`/its `Drop` (`:1339-1343`, `:1345-1371`, `:1373-1381`) are **not
compiled at all**, so both tests pass while exercising nothing. plan5's anchors (`scope.rs:847-858`,
`:1077`, `:1083`, `:1111`, `:2321-2323`, `:2375-2376`) are all STALE; the tests are
`src/scope.rs:2641-2668` and `:2695-2728`.

```
cargo test -p boyko-threadpool --release --features ke16-c-batch --lib -- --exact \
  scope::tests::spawn_batch_with_more_bodies_than_promised_still_runs_every_body_in_release \
  scope::tests::spawn_batch_drains_every_legal_promise_and_yield_pair
```

`--release` is load-bearing and correct: root `Cargo.toml:93-94` sets only `lto = "fat"` on
`[profile.release]`, so `debug_assertions` is off and the `#[cfg(not(debug_assertions))]` test at
`src/scope.rs:2642` is actually compiled. ⚠ **Read the output as `running 2 tests`, never as exit 0**
— plan5's command carried two bare filters and no `--exact`, and a rename then yields
`running 0 tests` and exit 0.

**2. The five receipts, in both arms.**

```
cargo test -p boyko-threadpool --test block_allocation_receipts -- --nocapture
cargo test -p boyko-threadpool --features ke16-c-batch --test block_allocation_receipts -- --nocapture
```

The default feature set is empty (`crates/boyko_threadpool/Cargo.toml:37`), mechanically pinned by
`tests/ke16_feature_scheme_census.rs:222`. One expected-constant set serves both arms because both
funnel through `prepare` → `Task::new_scoped` → `emplace` (`src/scope.rs:1059`; `:1137`/`:1151`;
`:1327`). ⚠ **In the DEFAULT build R1b is a duplicate of R1a**, since `spawn_batch` is literally
`for f in bodies { self.spawn(f) }` — **so the `ke16-c-batch` re-run is the load-bearing half of this
condition, not a formality: it is the only configuration in which R1b tests a second entry point.**

**3. The rewritten teardown tests, Miri, leak checking ON.** `src/task.rs` is gone; the scoped tests
are `src/task/scoped.rs:222-268` and `:270-309` (each opens a `ScopeBlock::new()` at `:229`/`:273`
and calls `block.free_all()` at `:267`/`:308`), with detached siblings at
`src/task/detached.rs:158-189` and `:190+`. The recipe is stated at `src/task/mod.rs:385-393` —
`MIRIFLAGS` **without** `-Zmiri-ignore-leaks`, which is already the tree's default
(`.cargo/config.toml:14-15` = `-Zmiri-tree-borrows`; CI's Miri job sets no `MIRIFLAGS` at all).

```
cargo +nightly-x86_64-pc-windows-gnu miri test -p boyko-threadpool --lib -- task::scoped::tests
```

⚠ **The filter is not optional.** Unfiltered, this recipe is red for a reason unrelated to the
property: `tests/miri_scope.rs:136-146` records that the residual "memory leaked" reports come from
crossbeam_epoch 0.9.18's at-exit TLS handle and retire queue, and the lib test binary reaches
crossbeam (`src/task/mod.rs:434`; `scope.rs`'s tests build pools). **Read the output as
`running 2 tests`.**

**4. Positives under the shipped recipe — CANNOT BE RUN AS WRITTEN.** The recipe citations are exact
(`tests/miri_scope_completion_protector.rs:114-119`, and `-Zmiri-preemption-rate=0` mandatory per
`:94-98`), but the condition requires `firings`, `overlaps` **and `block_overlaps`** recorded before
and after, and `block_overlaps` is recorded **nowhere** (§Sound). It becomes runnable when a sibling
of `assert_probe_armed` reads `boyko_threadpool::miri_block_frees_inside_a_release_window()`
(`src/lib.rs:362`) and adds it to the printed line at `:343-344`.

**5. The `ke16-w-count,ke16-a2` armedness re-check — CANNOT BE SATISFIED AS WRITTEN.** It is stated
"after 3z", and **3z was deferred rather than reverted**, so its written fallback does not apply. It
must be run in plan4's form on this tree:

```
MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-permissive-provenance -Zmiri-ignore-leaks -Zmiri-preemption-rate=0" \
  cargo +nightly-x86_64-pc-windows-gnu miri test -p boyko-threadpool \
  --features ke16-w-count,ke16-a2 --test miri_scope_completion_protector -- --nocapture
```

**Read the verdict off the `KE16-PROTECTOR-GATE-ARMED` line
(`tests/miri_scope_completion_protector.rs:343-344`), never off the exit code.** ⚠ **If it reads
`overlaps=0/2` there is NO discharge path**: re-tuning is refused in writing
(`src/scope.rs:139-146`, `src/task/detached.rs:62-69`) and 3z is not in the tree. **This is the one
condition with no remedy if it goes red, and the register carries it as an accepted risk rather than
a plan.** The reason it is a real risk and not a formality is written into the tree itself:
`src/task/detached.rs:42`, *"THE `debug_assert!` BELOW, AND THE RE-CHECK THIS COMMIT OWES"*, with
`:52-60` recording that since 3b a scoped spawn emplaces into the block and never reaches
`alloc_cell`, so **"the coupling no longer has the form it was measured in, and that is a re-check
owed rather than a coupling removed"** — including the measured before/after `overlaps=2/2` →
`overlaps=0/2`.

**6. The negatives — `scripts/tb_neg_gate.ps1`, 4/4 red with receipts. CANNOT BE RUN: nothing of 3c
exists** (§Ladder).

**7. loom M1/M1c, no atomic added, removed or reordered.** The conclusion **holds** and the condition
is discharged by re-running the loom leg — but two things in plan5's sentence are wrong. The
`cfg(loom)`/`cfg(miri)` exclusion is at `src/scope.rs:308-310`, not `:211-214`; and the atomics in
the tree were added by **3b, not 3z** — one static at `src/scope.rs:295-297` and two ops at
`:426-430`, all `#[cfg(miri)]`. `src/block.rs` has **zero** atomics (its only imports are `Layout`,
`Cell`, `ptr`/`NonNull`, `alloc`/`dealloc`/`handle_alloc_error`, `:76-79`), and
`src/task/{mod,scoped,detached}.rs` carry atomics only inside `#[cfg(test)]`. So **no production
atomic is added, removed or reordered.** Run the loom leg with `--cfg loom` added through
`cargo --config target.x86_64-pc-windows-gnu.rustflags=[…]` — which **joins** the ISA baseline
rather than replacing it, unlike `RUSTFLAGS` — over `tests/loom_pool.rs` (M1c at `:1534+`).
⚠ **The exclusion is a documented assumption, not a compile-time guard**: no
`cfg(all(loom, miri))` refusal exists anywhere in the crate. It holds by invocation discipline (loom
targets are `#![cfg(loom)]`, and nothing in the tree invokes miri with `--cfg loom`).

**8. `cargo check` under the A1 arms — N/A but cheap.**

```
cargo check -p boyko-threadpool --features ke16-a1
cargo check -p boyko-threadpool --features ke16-a1-fifo
```

CONFIRMED N/A: `src/tls.rs` is untouched by 3b and the three new `src/task/*.rs` files contain zero
`tls::`/`crate::tls` references. It cannot go vacuously green in a way that matters, because a
compile failure is the only outcome it claims to detect.

**9. `cargo check -p boyko-threadpool --features tb-neg-m2w` must FAIL — VACUOUSLY GREEN TODAY.** It
does fail, **for the wrong reason**: `tb-neg-m2w` is not a declared feature
(`crates/boyko_threadpool/Cargo.toml:36-118` declares `default`, `scheduler-trace` and eleven
`ke16-*`), so cargo exits non-zero with *"none of the selected packages contains these features"* and
**the condition's literal pass criterion is met while the arm it gates does not exist.** The
condition must assert the **diagnostic text** of the `not(miri)` `compile_error!`, not the exit code,
and it is meaningful only after 3c declares the feature and adds the refusal to `src/lib.rs` (which
today holds ten `compile_error!`s, `:82-126`, none naming `tb-neg-m2w`).

**10. §2.2's physics falsifier — CANNOT BE RUN ON THIS MACHINE NOW.** Both anchors are exact:
`crates/boyko_physics/src/solver/colored.rs:2817` (`scope.spawn_batch(n_waves, cuts().map(task))`,
inside `if boyko_threadpool::KE16_SPAWN_BATCH {` at `:2815`) and
`crates/boyko_physics/src/resources.rs:1879` (same `const` branch at `:1877`). The falsifier is
§2.2's withdrawn claim restated: `emplace` aligns to `align_of::<T>()` only — 8 for every shipped
body — so a bump makes cell *k* and *k+1* **certainly** adjacent where today two cells of one size
class merely **probably** share a line. Line sharing is not worse in kind, but it is more frequent in
degree. **Whether that regresses is UNVERIFIABLE BY READING.** **Settled by** a physics-dispatch timing
run on a verified-idle machine, against the physics rows of the campaign's own measurement layout
(`KE16-DESIGN-MEASUREMENT.md`) — but **the verification identified no bench target or profile for
it**, so the invocation is owed as well as the number. On the owner's workstation this is an
owner-scheduled measurement and not a CI leg. **No such number has been taken** (§NoTiming).

### ⚠ The vacuity survey was incomplete, and plan5's own framing is REFUTED

plan5 rewrote condition 1 because "an exit condition vacuous in the configuration it names is this
campaign's own catalogued shape", with the implication that the other nine do not carry it. They do:

* **Condition 9 carries the identical shape and is vacuously green today** (above).
* **Conditions 1 and 3 carry a second, different shape**: both are filter-based invocations with no
  `running N` assertion and no `--exact`, so a rename or a `cfg` drift produces `running 0 tests` and
  exit 0 — the trap plan5 §4 names by its own name.
* Conditions 2, 4, 5, 6, 7, 8 and 10 are clean of it.

**Condition 2's receipts are the best-defended of the ten and should be the template for the M2w
reader condition 4 still owes**: `tests/block_allocation_receipts.rs` has no file-scope `#![cfg(`,
its single test is unconditional (`:549-550`), R0 arms the cell bucket before R1 asserts zero in it,
R0b arms the chunk bucket before R2 predicts it, and the panic phase asserts `CHUNK_ALLOCS == 1`
specifically so that `CHUNK_LIVE == 0` cannot be green from emptiness (`:876-879`).

---

## §Corrections. What the plan said, and what is true

Consolidated so a reader auditing plan5 against the tree does not re-derive them. **14 claims were
REFUTED, 7 were STALE**, out of 55 checked; 32 were CONFIRMED and 2 are UNVERIFIABLE.

| plan5 said | true |
|---|---|
| §2.1/§6: pre-3b `Scope` at `scope.rs:748-766` | 16 B is right; anchor is `src/scope.rs:893-919` at HEAD (struct `:901-919`), `:957-1003` in the working tree |
| §2.1: the spawn path spans bytes **0..40** | **0..32** — `bump` never reads `n_chunks` |
| §2.1: "Drift is a build failure" of the layout claim | the size and align pins **do not cover the field ORDER** the one-cache-line claim rests on; no `offset_of` pin exists |
| §3: `__layout_receipt` publishes **five** literals; double asserts for `ScopedCell` only | **seven**, and the tree carries three ALIGN pins plan5 never mentions — leaving alignment to the test is the vacuity `src/lib.rs:558-563` names |
| §3: the instrument is **four `AtomicUsize`** statics | three `AtomicUsize` + one **`AtomicIsize`** (`CHUNK_LIVE`), six atomic statics in all |
| §3/§2.3: `alloc_cell` at `task.rs:334-348`, `handle_alloc_error` at `:345-347` | `src/task/detached.rs:74-90`, `:86-88` |
| §2.3: `schedule.rs:455` is a scope **per dispatch round** | **one scope per `Schedule::run`, per frame**; anchor correct, label wrong |
| §2.3: the schedule row's upper end is "**57 KiB**" | 57,344 B = **56.0 KiB** (57.3 kB decimal) |
| §2.3: the row's low end is "3.6 KiB" | **UNVERIFIABLE** — not re-derivable from the row's own inputs |
| §6: "**384 KiB** at the largest shape" | 384 KiB **payload**, **508 KiB requested** — and the clusters disagreed about whether the bare figure is acceptable (§What) |
| §6: doubling costs "**up to 2×**" | ~3× in the multi-chunk regime decaying to 2×, **plus a 4 KiB floor** — 85× for one 48 B cell, 512× for one 8 B cell |
| §6: "only the touched half becomes resident; on Windows the commit charge tracks the allocated figure" | readable half true; **OS halves NOT VERIFIED** |
| §2.4: `task.rs:78-85`, `:351-366`, `:381`, `:382`, `:407-437` | `src/task/mod.rs:117-140`; `src/task/scoped.rs:46-65`, `:83`, `:84`, `:101-164` |
| §4/D2: "no caller can form a REFERENCE into chunk memory" | **false as a statement about the crate** — the drop glue in `drop_unrun_scoped` (`src/task/scoped.rs:191`) hands the user's destructor a `&mut F` inside a chunk; safety is by ordering |
| §4/D5's frame enumeration | **incomplete**: `Task::drop` (`src/task/mod.rs:359-371`) also receives the chunk address, and the drop-glue reference is not named as a reference |
| §2.5: the window opens at `:246`, closes at "the slot store (`:257`)"; `fetch_sub` at `:599`/`:668`; `unpark` at `:620` | opens `:346-349`, **closes at a `compare_exchange` (`:368-375`), not a store**; `fetch_sub` `:805`/`:885`; `unpark` `:826` (and `:868`, *before* the decrement, on the external arm) |
| §2.5: `block_overlaps` "is printed alongside `overlaps=` on every run" | **zero occurrences in the crate** — the counter landed, the reader did not |
| §6: M1's 3/4 at `miri_scope_completion_protector.rs:284-289`; the other range is receiver × seed and "reads 4/4" | `:296-301`; the other range is `:88-93`, receiver × **probe** × **rate**, and its sixth cell reads "1/4 UB (seed 7)" |
| §Exit 1: `scope.rs:847-858`, `:1077`, `:1083`, `:1111`, `:2321-2323`, `:2375-2376` | `:1097-1108`, `:1339-1343`, `:1345-1371`, `:1373-1381`, `:2641-2668`, `:2695-2728` |
| §Exit 3: `task.rs:591-622`, `:650-676` | `src/task/scoped.rs:222-268`, `:270-309` |
| §Exit 7: the `cfg(loom)`/`cfg(miri)` exclusion at `scope.rs:211-214`; the atomics are 3z's | `:308-310`; the atomics are **3b's** (`:295-297`, `:426-430`) |
| §Exit 9: the command must FAIL with the `not(miri)` `compile_error!` | it fails because the feature is **undeclared** — vacuous |
| §Exit 1's framing: the other nine conditions do not carry the vacuity shape | conditions 9, 1 and 3 do |
| plan4: `MAX_CHUNKS = 16` = "268 MiB, three orders of magnitude of headroom" | **256 MiB**, **2.3×** headroom in chunks (16 vs 7) |

**One claim in the set is not a plan claim at all**: `size_of::<Task>() == 16` /
`align_of::<Task>() == 8` / `size_of::<Option<Task>>() == 16` appear nowhere in plan5. The tree
carries them itself, correctly, at `src/task/mod.rs:186-204`.

---

## §Instruments. Defects this campaign found in its own gates

Written here rather than in a footnote, because the next reader will hit them.

**1. ⚠ Exit condition 9 is green from an undeclared feature.** `cargo check --features tb-neg-m2w`
exits non-zero because cargo does not know the feature, which satisfies a "must FAIL" criterion while
the arm it gates does not exist. **A condition whose pass criterion is an exit code cannot tell a
refusal from an absence.** Fix: assert the `compile_error!`'s diagnostic text.

**2. ⚠ `tb-neg-m2w` is invisible to the feature census, in both directions.**
`tests/ke16_feature_scheme_census.rs:204-208` filters manifest features by `.starts_with("ke16-")`
(`:207`), so a `tb-neg-m2w` row is invisible to `the_declared_switch_set_is_exactly_the_designs_eleven`
**and** to `every_declared_switch_has_either_an_arm_or_a_refusal` (`:261`). plan5 §4's obligation to
"re-confirm at implementation time" is hereby **discharged in the safe direction** — the census will
not reject the new row — and that is exactly why `tests/tb_neg_m2w_arm_present.rs` is owed and why
condition 9 cannot be the only guard.

**3. ⚠ M2w's counter exists and is read by nothing.** An unmonitored counter is easier to mistake for
a monitored one than a missing counter is. `src/scope.rs:295-297` increments,
`src/lib.rs:362-363` exports, and no printer or assert consumes it — while §2.5 twice describes it as
a monitor.

**4. ⚠ The property that was bought is the field ORDER, and no `const _` asserts it.** Both pins on
`Scope` stay green under a reordering that puts the hot pairs on different cache lines (§Layout).

**5. ⚠ plan5's literal list leaves the ALIGNMENT half of the receipt bucket to the test binary** —
"a cell class free to move alignment leaves the receipt counting over an EMPTY bucket, and 'zero
allocations' comes back green from emptiness" (`src/lib.rs:558-563`). The implementation caught its
own plan here; the plan is the stale side.

**6. ⚠ Two exit conditions are bare libtest filters with no `--exact` and no `running N`
assertion** (1 and 3), which is the catalogued `running 0 tests` / exit 0 shape. By contrast the
receipts binary is defended by construction (§Exit, vacuity survey).

**7. ⚠ The `cfg(loom)` / `cfg(miri)` exclusion is an assumption, not a guard.** No
`cfg(all(loom, miri))` refusal exists; it holds by invocation discipline, which is enough for
condition 7 and is worth stating rather than inheriting.

**8. ⚠ In the default build R1b duplicates R1a**, so condition 2's `ke16-c-batch` arm is the only
configuration in which the second entry point is actually tested.

**9. ⚠ The chunk bucket's disjointness is one predicate term deep.** Remove `align == 64` and
`Buffer<Task>` walks straight into the bucket at 4096, 8192, … . And the residual — that no other
allocation in the process lands at `(2^k >= 4096, 64)` — **cannot be settled by reading at all**;
R0b and R2's exact equalities are the decision.

**10. ⚠ `cargo +nightly` resolves to MSVC on this box and dies in the linker with exit 1**, which is
indistinguishable from a red gate (`KE16-RESULTS.md` §0). Every Miri command in this file spells
`+nightly-x86_64-pc-windows-gnu`.

**11. ⚠ A citation rotted inside a file whose content did not change.** plan5's `:284-289` for M1's
3/4 was correct at `d647d930` and shifted +12 lines when 3y added a counter *above* it. This is the
argument for plan5 §4's own rule — **pin by file and function name, never by line number** — and
this register follows it for `assert_probe_armed`.

---

## §Owed. What this document does NOT contain

Named here rather than in a covering note, because **a reader who mistakes this register for a
finished campaign will ship a soundness downgrade whose execution gate does not exist.**

1. **M2w'S READER IS OWED, AND IT IS THE CAMPAIGN'S LARGEST OPEN RISK.** The lost clause (§Sound) is
   replaced by D1 + D2 + D3 + D5 **plus an execution gate over the one function D5 reduces to** —
   and that gate does not exist. The counter landed in 3b; nothing prints or asserts it. ~10 lines
   in a sibling of `assert_probe_armed`, on the template of condition 2's receipts.
2. **STAGE 3c IS NOT WRITTEN** — all eight items in §Ladder. Zero occurrences in the worktree. This
   also means **plan5's 3b+3c pairing is violated in the tree**: 3b's downgrade is in the working
   tree with a positive-only column.
3. **NO EXIT CONDITION HAS BEEN RUN FOR THIS REGISTER.** Six are discharge-ready (1, 2, 3, 7, 8, 10
   modulo the machine) and each carries its command in §Exit. Read every one by its `running N` line
   or its printed receipt, never by exit code.
4. **CONDITION 4 CANNOT BE DISCHARGED AS WRITTEN** (no `block_overlaps` reader) and **CONDITION 9 IS
   VACUOUS AS WRITTEN** (undeclared feature). Both need an edit before they mean anything.
5. **CONDITION 5 HAS NO FALLBACK.** It must be run in plan4's form because 3z was deferred rather
   than reverted, and if it reads `overlaps=0/2` there is no discharge path — re-tuning the yield
   count is refused in writing at two sites. **Carried as an accepted risk, not as a plan.** 3y's
   CAS substrate makes 3z a two-line change if the trigger fires.
6. **STAGE 3b IS UNCOMMITTED.** Seven `git status` entries. Nothing in this register describes
   committed code except stages 1, 3a and 3y.
7. **NO PERFORMANCE NUMBER EXISTS AND NONE IS OWED BY THE SHIP RULE** — the change ships on R1. Two
   figures a reader will want and must not take from here: the **+13-instruction / ten-`vmovups`
   empty-scope cost cannot be re-read on this tree** (`probe_empty_scope` does not exist here), and
   **exit condition 10's physics falsifier has not been measured** and needs a verified-idle,
   owner-scheduled window.
8. **TWO NUMBERS ARE UNVERIFIED AND ARE NOT PROMOTED**: the schedule row's **3.6 KiB** low end
   (settled by printing `self.systems.len()` in `Schedule::run`, or by tallying the 94 `add_systems`
   sites), and the **Windows commit-charge / residency** half of §6's growth sentence (settled by
   `GetProcessMemoryInfo` `WorkingSetSize` vs `PrivateUsage`, or perfmon Working Set / Private Bytes
   / Committed Bytes, around a probe that forces a ≥256 MiB chunk).
9. **THE FIELD-ORDER PIN IS NOT WRITTEN.** `const _: () = assert!(offset_of!(Scope, block) == 16);`
   costs nothing and closes the gap the two existing pins leave open.
10. **THREE WRITTEN ARGUMENTS IN THE TREE NEED RECONCILING WITH THEMSELVES**: D2's unqualified "no
    reference into chunk memory" (`src/block.rs:44-51`) against the drop-glue reference two files
    away (`src/task/scoped.rs:170-175`, `:191`); D5's frame enumeration, which omits `Task::drop` and
    does not name the drop-glue reference; and `Scope::drop`'s leak-safety claim, which survives on
    `abort_on_task_panic` (`src/worker.rs:183-218`) rather than on structure, without saying so.
11. **THE ACTIVATION LIST'S CROSS-REFERENCE IS OFF BY ONE STEP** — in both revisions, so it is
    carried rather than introduced (`src/task/scoped.rs:52-59`). Harmless; a reader will trip on it.
12. **PLAN5 ITSELF NEEDS TWENTY-THREE AMENDMENTS BEFORE ANYONE AUDITS AGAINST IT** — one
    per plan5 row of §Corrections' 24, the twenty-fourth being plan4's `MAX_CHUNKS = 16` figure
    (14 REFUTED + 7 STALE findings, plus the anchors they carry). It is not
    in this repository and is expected to be collected, so the amendments live here instead. The
    ones that change a *number* rather than an anchor: five literals → seven, 0..40 → 0..32, four
    `AtomicUsize` → three plus one `AtomicIsize`, "up to 2×" → ~3× plus a 4 KiB floor, 57 KiB →
    56.0 KiB, and 384 KiB → 384 KiB payload / 508 KiB requested.
13. **ONE DISAGREEMENT IS RECORDED RATHER THAN ADJUDICATED** — the 384 KiB sentence (§What). Two
    verification arms computed the same arithmetic and split on whether the bare payload figure may
    stand in a paragraph that names the allocated column. Both numbers travel together here; nobody
    picked a winner.

---

# §Run. 2026-09-08 — THE REGISTER WAS A READING; THIS IS THE RUN

Everything above was written without executing a single build command, and said so in its own
opening: *"every 'green' named below is **owed, not observed**."* This section is what happened when
the commands were finally taken, on an owner-verified idle machine (CPU 9.5 → 5.9 → **0.7 %** over
three one-second samples of `\Processor(_Total)\% Processor Time`; a process COUNT is not a load
receipt on this box — `tasklist` reports zero while PowerShell reports 268).

**Three of the register's own "not done" rows were already false when it was written**, because a
later session wrote stage 3c and did not amend the register. That is the same defect the register
was created to prevent, wearing the opposite sign: a document that under-reports what exists is as
stale as one that over-reports it, and a reader who trusts §Owed items 1, 2 and 6 would have rebuilt
work that was sitting in the tree.

| §Owed / §Exit row said | Actually in the tree, 2026-09-08 |
|---|---|
| 1. "M2w's reader is owed … the counter landed in 3b; nothing prints or asserts it" | **PRESENT.** `miri_scope_completion_protector.rs:243` reads `miri_block_frees_inside_a_release_window()`, and `block_overlaps=` is on the printed `KE16-PROTECTOR-GATE-ARMED` line at `:424` |
| 2. "STAGE 3c IS NOT WRITTEN — zero occurrences in the worktree" | **WRITTEN.** `tests/tb_neg_m2w_block_reference.rs` (the driver), `tests/tb_neg_m2w_arm_present.rs` (the census), `scripts/tb_neg_gate.sh` + `.ps1` (the driver pair), and the arm itself in `src/task/scoped.rs` |
| Condition 4 "CANNOT BE RUN AS WRITTEN — no `block_overlaps` reader exists" | **RUNNABLE**, by the row above |
| Condition 6 "CANNOT BE RUN: nothing of 3c exists" | **RUN AND DISCHARGED** — see below |
| Condition 9 "VACUOUSLY GREEN TODAY — `tb-neg-m2w` is not a declared feature" | **NON-VACUOUS.** Declared at `Cargo.toml:138`; the `not(miri)` refusal is `src/lib.rs:171-173` |

## Exit condition 6 — DISCHARGED, 4/4 seeds, receipts committed

`scripts/tb_neg_gate.sh`, one Miri process per seed over `(0 1 7 15)`, `-Zmiri-tree-borrows
-Zmiri-disable-isolation -Zmiri-permissive-provenance -Zmiri-ignore-leaks -Zmiri-preemption-rate=0`:

```
seed 0 : RED, attributed (exit=1)     seed 7  : RED, attributed (exit=1)
seed 1 : RED, attributed (exit=1)     seed 15 : RED, attributed (exit=1)
red on 4/4 seeds
```

Receipts in `docs/threadpool/receipts/tb-neg-m2w-<seed>.stderr`. All four are **identical to the
column**: UB at `block.rs:341` inside `ScopeBlock::free_all`, accessed tag born at `block.rs:482:28`
(`grow`'s `alloc`), protected tag born at `scoped.rs:303:17` (`finish_neg`'s `_cell` parameter), and
the backtrace `free_all` ← `<Scope as Drop>::drop` ← `PoolInner::install`. Determinism across seeds
is what `-Zmiri-preemption-rate=0` is in the recipe for, and it is now observed rather than
predicted.

## ⚠⚠ THE FIRST RUN READ `red on 0/4` — AND THE PROPERTY WAS PERFECT ON ALL FOUR

This is the finding of the day and it belongs at the top of the section, not in a footnote.

The gate's first invocation reported `NOT RED FOR THE DECLARED REASON` on every seed, with
`ub=yes kind=yes protector=no accessed=yes`. **Nothing was wrong with the arm, the recipe, the
soundness argument or the property.** Receipt grep 3 required the arm's function NAME inside the
three-line window after `the protected tag <N> was created here`, on an argument the script states
explicitly and which is correct as far as it goes: a spanned `help:` renders header, `-->`, gutter,
snippet, so the snippet carrying `finish_neg` lands at +3.

**Miri does not render these two `help:` sub-diagnostics with a snippet.** It prints the header and
the `-->` location, and stops. `grep -c finish_neg` over any of the four receipts returns **0**. The
predicate was **unsatisfiable by construction** — the gate could not pass, on any input, ever.

So this is the family's mirror image. The catalogue in this repository is full of gates that could
not FAIL; this is a gate that could not PASS, and it is more dangerous than it looks, because its
red is indistinguishable from the finding it exists to report. A reader who trusted it would have
concluded that M2w had been *refuted* — that the shipped `run_scoped` path's positive column had
stopped being falsifiable — and gone looking for a soundness defect that does not exist. The
script's own `TIGHTEN THIS ONCE THE FIRST RECEIPT EXISTS` note is what kept the cost to one hour:
the author knew the predicate was a guess about rendering and said so in the file.

**The repair, and why it is not a rubber stamp.** The receipt does carry the location, so identity
is checked on that — with the line **derived from the arm's own source on every run**, in all three
readers (`tb_neg_gate.sh`, `tb_neg_gate.ps1`, and the census in `tb_neg_m2w_arm_present.rs`), and
never written down. The distinction matters, because the original message forbade line numbers on
the grounds that *"a citation into this crate rotted once inside a file whose content had not
changed"* — a citation rots when a number frozen in one file outlives an edit to another, and a
number recomputed from the file it points into cannot. If the signature moves, the rule moves with
it; if `_cell: &ScopedCell<` stops occurring exactly once in `scoped.rs`, all three readers ABORT
rather than match something else.

Canaried before it was believed, in the order this repository had to learn the hard way — the
predicate must be shown able to say **no**:

| rule tried against the real seed-0 receipt | verdict |
|---|---|
| protector at `scoped.rs:303:` (the derived line) | MATCH |
| protector at `scoped.rs:302:` / `:304:` / `:1:` | refuse, refuse, refuse |
| accessed at `block.rs:482:` (the derived line) | MATCH |
| accessed at `block.rs:481:` | refuse |
| accessed at `block.rs:724:` — the OTHER `alloc` in the same file, in its test harness | **refuse** |

The last row is the one that shows the tightening is real: the previous rule asked only for
`block.rs`, which that line satisfies.

**One predicate was ADDED, not just repaired.** Grep 4 says *what* was freed; new grep 5 says *who*
freed it — the receipt must name `ScopeBlock::free_all` and a `Drop::drop` frame. Stage 3b's whole
replacement argument is that the chunk dies in `free_all` **called from `Scope::drop`, immediately
after the join**; a UB report naming the right allocation from some other reclamation path would
otherwise wear this gate's receipt while saying nothing about the claim.

## The driver was RED IN EVERY CONFIGURATION, and now carries the tree's own remedy

`a_protector_over_chunk_memory_held_across_the_release_is_caught_by_tree_borrows` asserts
`UNDER_MIRI` natively and panics `arm not built` under a Miri run without the feature; under the
gate's own invocation its success **is an abort**. There is therefore no configuration in which the
binary is green, and as written it stood permanently red in `cargo test --workspace`.

The file defends that red in a comment, and the defence is sound against the objection it answers
(`const { assert!(…) }` would be worse — a build failure, not a red). It does not answer this one.
A known-red target is this repository's most-measured defect: it teaches readers to skip a line, and
three reds hid behind one for 87 commits (CLAUDE.md, `--no-fail-fast`).

**The tree already had the answer and had already used it twice.** `#[ignore]` with a written reason
is the sanctioned form — `tests/ignore_reasons_census.rs` fails the build on a bare or empty one —
and the crate's own suite prints, one line from the driver,
`worker_spawned_wave_reaches_at_least_half_the_workers ... ignored, deferred: … RED in the default
build BY DESIGN`. So the driver now carries
`#[cfg_attr(not(all(miri, feature = "tb-neg-m2w")), ignore = "miri-arm: …")]`.

The `all(…)` is load-bearing and `not(miri)` alone would have been wrong: a plain `cargo miri test`
builds default features, where the arm-not-built panic is equally undecidable. Both driver scripts
now pass `--include-ignored`, and the census requires all three facts — the exact cfg, a non-empty
reason, and the flag in both scripts — so the ignore cannot decay into a disappearance.

Nothing is given up. The two assertions still fire for anyone who runs the binary out of
configuration, and the NATIVE gate over this property was never this test: it is
`tb_neg_m2w_arm_present.rs`, unignored, which asserts the arm is in the source, that both scripts
stay in step, and that four committed receipts are red for the declared diagnostic.

## Gate ladder as actually run

| leg | verdict |
|---|---|
| `cargo check -p boyko-threadpool --all-targets` | **PASS** |
| `cargo test -p boyko-threadpool --all-targets --no-fail-fast` | **PASS** — `CARGO_EXIT=0`, 156 passed, 0 failed, 3 ignored |
| `scripts/tb_neg_gate.sh` (exit condition 6) | **PASS** — 4/4 red, attributed |
| `cargo clippy --workspace --all-targets -- -D warnings` | ⚠ **RED, INHERITED AND NOT FROM THIS WORK** — see below |

⚠ **The clippy red is a FALSE POSITIVE in clippy 0.1.97 and belongs to neither 3b nor 3c.** Five
sites in `crates/boyko_diag/src/lane.rs` and `crates/boyko_shaderdsl/src/emit/mod.rs` raise
`initializer for thread_local value can be made const` on initialisers that **already are** `const`
— e.g. `static LANE: Cell<u16> = const { Cell::new(LANE_UNCLAIMED) };`. Neither file appears in this
work's `git status`, so the lint result is independent of it. Recorded rather than repaired:
silencing a false positive in two unrelated crates is its own unit, and it must not be smuggled into
this one. It does mean **the clippy leg of the four-command ladder is currently red workspace-wide**,
which any later reader should know before quoting a "green" from this campaign.

## What is still owed after this run

The register's §Owed items 3, 4, 5, 7, 8, 9, 10, 11, 12 and 13 stand as written. Items 1, 2 and 6
are discharged above. The exit conditions other than 6 have **not** been run in this pass and remain
as §Exit describes them, with conditions 4 and 9 now reachable where they were not.
