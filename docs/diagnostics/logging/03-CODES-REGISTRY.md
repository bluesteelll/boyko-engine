# Logging — the diagnostic-code registry, the walker, and the migration

<!-- CONTRACT
provides: logging/registry-and-walker
assumes:  seam/diagnostic-code-space
assumes:  substrate/lane-write-sites
assumes:  logging/goal-and-audiences
assumes:  logging/budgets-and-invariants
-->

**Carved from `docs/LOGGING-SYSTEM-PLAN.md` (v4)** — Decisions 6, 7 and 19 in full; §Integration's New / Migration ledger / Behaviour changes / Enforcement / Compatibility. Diff against the monolith until it is retired.

**Two facts about *this* file, because they are load-bearing.** (1) It lives in `docs/diagnostics/**`, which is inside the TEXT corpus check 4 scans — so the bare-number rule below applies to it with full force, and every prefixed literal it carries names a code the registry actually registers. (2) `docs/diagnostics/` **did not exist** when v4 was written; the corpus split created it. Both consequences are recorded in the corpus section rather than left for a reader to discover by reddening a gate.

---

## Decision 6: One diagnostic-code registry, kept honest by eight mechanical checks over a SPECIFIED walker *(fixes F5, F20)*

**What.** `crates/boyko_log/src/codes.rs` holds a single `codes! { … }` invocation generating: a `pub const` per code, a **dense** `static DIAGNOSTICS: [DiagInfo; N]` sorted by number, a dense `code_idx` per code (the `RATE` index — M12), and `explain()`. A literal `"boyko-…"` outside the registry is a build failure. Class is a **type** property: `warn!` takes `WarnCode`, `error!` takes `ErrorCode`, `PanicCode` is distinct — a class mismatch does not compile.

**The checks** live in `crates/boyko_log/tests/code_registry.rs` — an **integration** test, because `cargo test --workspace --lib` does not build `tests/`, a blind spot that cost this repo four commits.

### The walker: ONE pass, THREE streams, and every check names the streams it consumes *(fixes F5)*

v2 specified no walker, and its checks 3 and 6 then required **opposite** behaviour from the one it did not specify. Measured in this tree, and **re-measured this session against HEAD**:

- `crates/boyko_ecs/src/ecs/core/app/app.rs` contains **24** occurrences of `boyko-B1802` *(re-confirmed: exactly 24)*; **18** are inside `/// # Panics` doc comments (`:267`, `:283`, `:303`, `:333`, `:365`, …), **1** is the panic message string (`:867`), and **5** are `#[should_panic(expected = …)]` inside the in-`src` `#[cfg(test)]` module (`:898`-`:939`). *(The review said 28 doc-comment sites; the measured number is 18. The finding stands — the count does not.)*
- `crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs` mentions `boyko-B9001` / `B9002` / `B9004` / `B9101` in doc comments, plus a `boyko-B900x` **pattern** at `:302` that is not a code at all. **Correction to v4's line list**: v4 cited `:302,304,309,314,317,318`; the measured set is `:302` (the `B900x` pattern), `:304` (`B9001`), `:309` (`B9101`), `:314` (`B9001`), `:317` (`B9002`), `:318` (`B9004`), **`:320` (`B9005`)** and **`:704` (`B9101`)**. Two sites were omitted. The finding is unaffected — a doc-comment scan still satisfies check 3 vacuously — but a line list one entry short is the shape of defect this corpus exists to stop, so it is corrected rather than carried.
- `crates/boyko_ecs/src/ecs/core/system/params/diagnostics.rs:46` — `/// Diagnostic code: \`boyko-B0002\`.` *(re-confirmed at `:46`)*.

So: a substring scan that includes comments makes check 3 (no orphans) satisfiable by writing a `# Panics` line — v1's vacuity relocated from `.md` into `.rs` — while a scan that excludes comments makes check 6 (panic-class placement) red on those same 18 sites the day L2 lands. Both cannot hold under one unspecified walker.

**The walker is specified.** One pass over each `.rs` file produces three disjoint streams, and it is the **same walker** that backs `print_census.rs`, so its `#[cfg(test)]`-region rule and its `src/bin/` exclusion are written once and exercised by two tests:

| Stream | Contents | Consumed by |
|---|---|---|
| **CODE** | source text with `//`/`///`/`//!` line comments, `/* */` block comments, string/char literals and `#[cfg(test)]` regions removed — **including whole files reached by a `#[cfg(test)]`-gated `mod` declaration, per the cross-file rule below (B7)** | checks 3, 3b, 6, 7 and the print census |
| **LIT** | the contents of string and char literals only | check 4 |
| **TEXT** | the whole unstripped file, plus the **explicit doc directory list** below — **`docs/archive/**` is NOT in it (B6)** | checks 0, 4 |

Check 3 matches a **standalone identifier token** (`B1802`, or the path form `codes::B1802`) in CODE — never a substring of `boyko-B1802`, which after stripping does not exist in CODE at all. Check 4 matches the `boyko-[BEW]\d{4}` **literal** in LIT ∪ TEXT. Check 6 runs over CODE, so doc mentions and the in-`src` `#[cfg(test)]` `should_panic` strings are invisible to it. The walker is deliberately not a Rust parser; its failure mode (a `#[cfg(test)]` block over-reaching to end-of-file) is the same one `scripts/check_hotpath_exceptions.py:164-201` already documents and accepts for the same reason — verified: `cfg_test_spans` begins at `:164` and its own docstring records "Brace counting is naive about braces inside strings and comments."

### The `#[cfg(test)]` rule is CROSS-FILE, because the corpus is *(fixes B7)*

v3's rule was within-file, and the ledger then claimed it excluded 23 in-`src` test prints. Measured against the tree, only **16 of the 23** are excludable that way — **all four counts re-confirmed this session**:

| File | Prints | How it is gated | Within-file rule sees it? |
|---|---|---|---|
| `crates/boyko_rhi_vulkan/src/compute/tests.rs` | 16 | `#[cfg(test)]` **inside** the file (`:6`, `:138`, `:257`, `:872`, `:1610`, …) | **yes** |
| `crates/boyko_sdf_math/src/brick/tests.rs` | 3 | parent: `crates/boyko_sdf_math/src/brick.rs:1829-1830` — `#[cfg(test)]` / `mod tests;` | **no** |
| `crates/boyko_physics/src/solver/colored_tests.rs` | 4 | parent: `crates/boyko_physics/src/solver/colored.rs:3198-3200` — `#[cfg(test)]` / `#[path = "colored_tests.rs"]` / `mod tests;`. The file's only `cfg(test)` match, `:2569`, is inside a **comment** | **no** |

So 7 of the 23 would be classified as production sites and `print_census.rs` would fail on them — forcing test-only sites into `print_allowlist.txt`, which is exactly the allowlist-laundering this design says it prevents.

**The rule, stated so it can be implemented without a Rust parser.** In one pre-pass the walker collects, from every `.rs` file, every declaration matching `#[cfg(test)]` (or `#[cfg(any(test, …))]`) followed within the next two attribute/whitespace lines by an optional `#[path = "REL"]` and then `mod NAME;`. Each such declaration marks a **file** as test-only: `REL` when `#[path]` is present, otherwise `NAME.rs` or `NAME/mod.rs` resolved against the declaring file's directory. Marked files contribute **nothing** to CODE and nothing to the print census. `#[cfg(any(test, …))]` is treated as test-only for exclusion purposes and **is listed by name in the walker's report**, so a file excluded because of a `feature`-plus-`test` disjunction is visible rather than silently absent; the engine has no such site today and a new one must be seen. `#[cfg(all(test, …))]` is also test-only. `#[cfg(not(test))]` is not.

**Arithmetic re-derived against the rule that will actually run**: `179 − 58 (CLI bins) − 23 (test-only files: 16 within-file + 7 cross-file) = 98` occurrences to disposition — the same 98, but now reachable by the specified rule rather than by a rule that would have missed 7. After S1 removes the 20 measurement rows the walker's site denominator is **≤ 78** (§Migration ledger).

> **179 and 180 are ONE census under TWO grep conventions, not drift — and the earlier claim that they were drift is withdrawn.** This note previously read "One site has appeared since v4's measurement." **That cause is false and now provably so.** The documented command counts matching **LINES** (`grep -rEn … | wc -l` ⇒ **179**); `grep -o` counts **MATCHES** (⇒ **180**); the file count is **36** either way. The entire one-match delta is a single line carrying two matches: `crates/boyko_render/src/texture.rs:734`, `/// A short label for diagnostics (\`eprintln!\`/\`println!\` notes).` — a **doc comment**, not a call site, and precisely the class this ledger's own "Prose mentions inside comments" row excludes from the denominator by rule. Measured at `5ebcd58` (the commit that first wrote these plans), `053f6c9`, `7a323d2` and HEAD, `git grep` returns **lines = 179 / matches = 180 / files = 36 at every one**, and `git show 5ebcd58:crates/boyko_render/src/texture.rs` line 734 is byte-identical to today's. **Nothing has appeared.** The 58 / 16 / 3 / 4 splits are convention-independent (checked per file: 58/58, 16/16, 3/3, 4/4 lines vs matches), so the delta lands nowhere but the residual, and it lands there as a non-site. The note is kept, with the true explanation, because "179" is quoted in three places in this corpus and a reader who re-runs the grep with `-o` must find the discrepancy explained rather than fresh.

### The TEXT corpus is an explicit directory list *(fixes B6)*

Measured by v4 (`grep -roE 'boyko-[BEW][0-9]{4}' docs --include='*.md'`): 75 occurrences across 13 files. **Re-measured this session: 97 occurrences across 13 files** — `docs/LOGGING-SYSTEM-PLAN.md` **61**, `docs/archive/**` **29 across 10 files**, `docs/PROFILING-SYSTEM-PLAN.md` **4**, `docs/SYSTEMS.md` **3**. The file count is stable; the two plan documents grew their own literal counts (41 → 61 and 2 → 4) between v4's measurement and this carve. *(The round-3 review said 70 and the orchestrator addendum said 75 with an archive share of 27. The total is a moving quantity because two of the scanned files are the plans themselves; the **archive share is 29 across 10 files** and has not moved.)*

**This corpus now adds to that count.** The carve created `docs/diagnostics/**`, which is in TEXT's directory list, and the carved files carry prefixed literals. Two consequences, both new at the split:

1. **The number is now self-referential by design.** A gate that asserted a *total* would red on every commit to the corpus. Check 0 asserts only non-emptiness (`files_scanned ≥ 500`) plus a pinned sentinel; check 4 asserts *resolvability*. Neither counts. That is not an accident of the original design and it must not be "improved" into a count.
2. **Check 2's target and the corpus share a directory.** Check 2 looks for `docs/diagnostics/<code>.md`; the corpus files are `README.md`, `SEAM.md` and three subdirectories, none of which matches `[BEW]\d{4}\.md`. There is no collision today, and check 2 must resolve its target as an **exact top-level filename**, never a glob over the directory, so that adding a corpus page can never be mistaken for adding a code page.

`docs/archive/` holds completed-phase planning documents that **must never be edited again**, and it contributes three codes that exist in **no source file and no current document**: `B9000`, `B9003` and — a case neither the review nor the addendum named — **`W9003`** (`docs/archive/PHASE-15-PLAN.md:471`). *(Written here **without** the `boyko-` prefix, deliberately: see the self-reference paragraph below. This file is inside the corpus check 4 scans.)* Seeding those as `Pending(rung)` is wrong: `Pending` promises a future emitter, and these promise nothing. **Re-measured archive distribution**: `B0002` ×6, `B1801` ×3, `B9000` ×1, `B9001` ×10, `B9002` ×2, `B9003` ×2, `B9101` ×3, `W1501` ×1 — 29 across `PHASE-9-PARALLEL-SCHEDULER-PLAN.md` (7), `PHASE-15-PLAN.md` (6), `PHASE-8A-SYSTEMPARAM-PLAN.md` (4), `PHASE-21-RESULTS.md` (3), `PHASE-15-RESEARCH.md` (3), `PHASE-18-RESULTS.md` (2), `PHASE-X.A-RESULTS.md` (1), `PHASE-X.A-RESEARCH.md` (1), `PHASE-15-RESULTS.md` (1), `PHASE-13-ROADMAP.md` (1). Two consequences:

1. **TEXT's corpus is an explicit directory list**, written in the test and not inferred: `docs/*.md` (top level), `docs/diagnostics/**.md`, `docs/ru/*.md`, `book/src/**.md`. **`docs/archive/**` is excluded**, with the reason in the test's own comment. Check 0's non-empty assertion and its pinned sentinel (`boyko-W1501`) still apply to the reduced corpus, so a mis-resolved root is still caught.
2. **A `Historical` row status exists** for the case where an archived path is ever re-included, or where a code appears in a frozen artifact this repository will not edit: zero emitters permitted, **no `docs/diagnostics/` page required**, and it never becomes `Live`. Check 3b (`Pending ⇒ 0 emitters`) applies to `Historical` too; check 3c (`Pending == 0`) does not, because a `Historical` row is not migration debt. No row is `Historical` at L2 — the three archive-only codes are simply outside the corpus — and the state is defined now so that re-including a directory is a one-line change and not a redesign.

**This document is itself in the corpus, and that has a consequence v3 missed.** v3's check-4 row spelled its illustrative red state as the **full prefixed literal** for code W9003 — a code that is real, unregistered, and present in `docs/archive/PHASE-15-PLAN.md:471`. Because check 4 matches the `boyko-`-anchored pattern over TEXT, and TEXT includes `docs/*.md`, v3's own planning document would have **red check 4 permanently, on itself, from the day the gate was armed**. The rule this corpus now obeys, and states so the next author does not undo it:

> **A planning document may name a code by its bare number-and-class (`W9003`, `B9000`, `B9003`, `E0115`) but may write the prefixed literal only for codes the registry actually carries.**

The gate log demonstrates check 4's red state by writing the prefixed literal into a **scratch file inside the corpus**, then deleting it — never by carrying it in a committed document. Every prefixed literal remaining in this file names a code this plan registers. **The rule now binds all 22 corpus files**, not just this one, because all of them live under `docs/diagnostics/**`.

### Doc-page debt nobody counted *(B6, measured)*

`boyko-B9004` (5 occurrences) and `boyko-B9005` (7) exist in `crates/**/*.rs` and in **no document at all**. Check 2 requires `docs/diagnostics/<code>.md` with three named sections for every `Live` row, so these two owe pages that do not exist and that v3's L2 exit criterion did not count. They are named in L2's line items with their rung: both are `Live` from L2 (they have emitters today), so both pages land **at L2**, in the same commit as the registry rows.

> **Precision the phrase needs, verified this session.** "In no document at all" is true of the **prefixed literal**, which is what check 4 matches: no `.md` anywhere contains `boyko-B9004` or `boyko-B9005`. Both **do** appear as **bare** identifiers, in `docs/archive/PHASE-15-PLAN.md:443,445,565,567`, `docs/archive/PHASE-15-RESULTS.md:31` and `docs/SYSTEMS.md:1483`. The doc-page debt is unaffected — a bare mention is not a page — but the sentence is qualified here so that a reader who greps bare numbers and finds hits does not conclude the record is wrong.

### `Live` vs `Pending`: how L2 commits alone on a grandfathered corpus *(fixes F20)*

L2 seeds the registry with the 9 existing codes, but the *identifiers* `codes::B9001`, `codes::W1501`, … do not appear in CODE until L6/L7/L8 migrate the emitters. A check-3 that scans identifiers therefore reds at L2, and one that scans literals is vacuous. Each registry row carries a status:

- **`Live`** — check 3 requires ≥1 CODE occurrence, **and check 2 requires its doc page**. Register a `Live` code and emit it nowhere ⇒ **red**.
- **`Pending(rung)`** — check **3b** requires **zero** CODE occurrences. Emit a `Pending` code ⇒ **red**, which forces the row to be flipped to `Live` in the same commit that lands the emitter. A `Pending` row cannot rot silently, because the day it acquires an emitter it reds. **Check 2 does NOT cover `Pending` rows** *(S6)*: a `Pending` row owes its `docs/diagnostics/<code>.md` in the same commit that flips it to `Live`. Requiring the page at seeding time would owe L2 eighteen pages for codes with no emitters — doc-rot manufactured by a gate.
- **`Historical`** — zero emitters permitted, **no doc page required**, never becomes `Live` (B6). For codes that exist only in frozen artifacts this repository will not edit. Check 3b applies; check 3c does not.
- Check **3c**, armed at L8c only: `Pending` count == 0. This is the migration's real exit criterion, and it is one integer. `Historical` rows are excluded from it by definition.

### The eight checks

Numbered **0 through 7**. Checks `3b` and `3c` are legs of check 3, not additional checks — the count is eight, and `codes_tidy!` generates all eight for a downstream table.

| # | Check | Stream / corpus | Red state that must be demonstrated once |
|---|---|---|---|
| 0 | **Corpus is non-empty**: `files_scanned ≥ 500`, and the pinned sentinel `boyko-W1501` is found | TEXT | point the walker at a wrong root → red |
| 1 | Numbers strictly increasing ⇒ no duplicates (also a `const _: () = assert!`). **This is why two rows may never share a number even across classes** — `W0110` and a hypothetical `E0110` would break it, and `DIAGNOSTICS` is dense with `index == code_idx` | registry | add a duplicate |
| 2 | `docs/diagnostics/<code>.md` exists, non-empty, has `## What happened` / `## Why` / `## How to fix`. **`Live` rows only** *(S6)*; `Pending` and `Historical` are exempt. **Exact top-level filename, never a glob** — the corpus now shares that directory | `docs/diagnostics/` | delete a section heading; **or flip a row to `Live` without its page** |
| 3 | **No orphans**: every `Live` code's identifier appears ≥1× as a standalone token | CODE, excluding `codes.rs` | register a `Live` code, emit it nowhere |
| 3b | **No premature emitters**: every `Pending` **or `Historical`** code's identifier appears **0×** | CODE | emit a `Pending` code without flipping its row |
| 3c | **Migration complete** (armed at L8c): `Pending` count == 0. `Historical` excluded | registry | leave one row `Pending` |
| 4 | **No undeclared**: every `boyko-[BEW]\d{4}` literal resolves to a registry entry | LIT ∪ TEXT (the explicit directory list; **`docs/archive/**` excluded**) | write the literal form of `W9003` into a scratch file inside the corpus. *(The full literal is deliberately not written in this committed document: it is a real archive code with no registry row, so carrying it here would red check 4 permanently — the self-referential failure v3 shipped.)* |
| 5 | Every `Live` `W`/`E` code is **named by** ≥1 test, with `tests/untested_codes.txt` (a **data file**, excluded from its own scan) checked **in both directions** | ~~`crates/**/tests/**`, `#[should_panic(expected=`~~ **ALL test code** — the walker's test-only set plus each production file's tail from its first `#[cfg(test)]`, minus `codes.rs` and `code_registry.rs`. **Re-specified at L6 against the tree**: the narrow corpus would have called thirteen genuinely-tested profiling codes untested, because their tests are `#[cfg(test)] mod tests` blocks inside `src/`. And "observed" is written as **named**, because a text scan cannot tell a test that drives the condition from one that mentions the code — the proxy is stated in the check's own failure message | allowlist a code that has a test |
| 6 | ~~Panic-class `B` codes appear only inside a `#[cold] fn … -> !` or a `panic!`~~ **A `B` code is never an argument to an emission macro** | CODE | emit a `B` code from a `warn!` |
| 7 | Every `LogTarget` impl in the workspace resolves to a `targets!` row or a `define_target!` expansion | CODE | hand-write a `LogTarget` impl |

> **Check 6's form is FALSE of a correct tree, and L6 measured it.** `ScheduleBuildError` is
> deliberately dual-purpose — `ScheduleBuilder::build` panics with `e.formatted()` while
> `try_build` returns the same value as an `Err` for a tool or library caller — so
> `B9001`/`B9002`/`B9004`/`B9005` necessarily live in a `String`-returning method that also feeds
> `Display`. Enforcing "only inside a `panic!`" would have required deleting the recoverable API or
> reverting the codes to string literals. What the rule is **for** survives whole, and it is this
> row's own red state: a `B` code is a broken invariant, and routing one through an emission macro
> makes it a line in a log the process then continues past. What the narrowed check cannot claim —
> that every `B` code reaches a panic — is written into the check itself.

**Why the corpus rules changed.** v1's check #3 was vacuous because check #2 *mandates* a doc file naming the code and v1's scan included `.md`. v2 narrowed the corpus to `.rs` and reintroduced the same vacuity through comments. v1's check #5 was self-defeating: the allowlist named identifiers and lived inside the file being scanned. Check #0 closes the third failure in the same family — a walker that resolves its root badly scans zero files and reports zero orphans, green. rustc's tidy pins a sentinel for exactly this reason.

**What these checks CANNOT claim.** They are engine-scope. They prove nothing about a game's or a mod's registry — which is why `codes_tidy!` (Decision 19) generates the same eight checks over a caller-supplied root and prefix, and why G9's assertion message says so in the failure text rather than in this document.

**Prior art.** None of Bevy / flecs / UE / Unity / spdlog / Quill / NanoLog ships a code registry. The prior art is compilers: rustc's numbered codes with a mandatory long-form `.md` and its eight tidy checks; Clang's named groups; MSVC's opaque numbers with per-code pages. rustc's experience is that the number is worthless without the mandatory explanation *and* the orphan check.

### Block map

Defined *around* **measured** existing occupancy, because codes are never renumbered. The "occupied today" column is not an assumption: it is `grep -roE 'boyko-[BEW][0-9]{4}' crates --include='*.rs'`, **re-run this session against HEAD**, which returns **89 occurrences and exactly 9 distinct codes** — `B0002` (24), `B1802` (24), `B9001` (11), `B9101` (7), `B9005` (7), `B9004` (5), `B9002` (5), `B1801` (4), `W1501` (2). That set **is** the "9 grandfathered codes" L2 seeds. **Both figures reproduce exactly**; nothing is inherited.

| Block | Domain | Occupied today |
|---|---|---|
| `00xx` | ECS core / system params | `B0002` |
| `01xx` | `boyko_log` itself | `W0103` (L4) |
| `02xx` | `boyko_threadpool` | `E0201` (L6) |
| `03xx` | memory (`VmReservation`, `ComponentPool`) | new |
| `04xx`–`09xx` | components, query, change detection, events, assets, serialize — **one block each, in that order**, which is what fixes `05xx`=query, `07xx`=events, `08xx`=assets | `W0501`, `B0502` (query, L6) · `W0701` (events, L6) · `E0801` (assets, L6) |
| `11xx`–`14xx` | input, scene, physics, math/sdf | new |
| `15xx` | schedule sets & ordering | `W1501` |
| `18xx` | app / plugins | `B1801`, `B1802` |
| `20xx`–`27xx` | RHI, RHI-Vulkan, render, shaderdsl, UI, fontbake, image, GPU columns | `E2101` (L7a) · `W2102`, `E2103`, `W2104`, `W2105`, `W2106` (L7b) — all six `boyko_rhi_vulkan`'s; `2106` is **not** in the ledger and was minted at L7b when the seven-site `E2103` split in two |
| `30xx` | host / runner | new |
| `90xx` | schedule **build** (historical) | `B9001`, `B9002`, `B9004`, `B9005`; **`B9003` permanent gap**; `B9000` and `W9003` appear only in `docs/archive/**`, which is outside the corpus (B6) |
| `91xx` | world binding | `B9101` |
| **`92xx`** | **profiling** — reserved at **L2** *(S6)* | none. **Measured**: the `9xxx` band is already occupied by `B9001`/`B9002`/`B9004`/`B9005`/`B9101`, but `92xx` itself is free **in source** — zero `92xx` literals under `crates/` or `src/`. The profiling plan asserted availability without checking; this row records the check |

The `15xx`/`90xx` split is a historical artifact, documented as such, and **must not be tidied**: renumbering would break the book, the `#[should_panic]` assertions and the never-reuse rule simultaneously.

### The `92xx` reservation, and why it lands at L2 and not later *(S6)*

`docs/PROFILING-SYSTEM-PLAN.md` already contains prefixed `92xx` code literals, and check 4 scans `docs/*.md`. So the day L2 arms its checks, an already-committed document reds check 4 unless the rows exist.

> **Re-measured this session, and the case is now stronger than v4's.** v4 counted "two literals in the profiling plan" (`boyko-W9207` at `:200`, `boyko-E9204` at `:376`). Today the prefixed `92xx` population across `docs/**.md` is **eight**: `docs/PROFILING-SYSTEM-PLAN.md` ×4 (`W9207` ×2, `E9204` ×2), `docs/LOGGING-SYSTEM-PLAN.md` ×4 (`W9207` ×3, `E9204` ×1) — **plus** the carved corpus, which already carries `boyko-W9207` in two of this area's own files (`logging/sink-lifecycle` and `logging/ladder`) and will carry more as the profiling files land. The literal count is **growing**, in **more files**, in a directory that is **inside the TEXT list**. Deferring the reservation past L2 makes the red bigger every commit.

L2 therefore seeds **all 18 `92xx` rows as `Pending(<profiling rung>)`** (eighteen: `W9201`..`W9218`, consecutive with no gap, adjudicated in `seam/diagnostic-code-space` — this file previously said 17, one short, and a reservation one row short reds check 4 on exactly the code it forgot), owning no doc pages (check 2 is `Live`-only) and no emitters (check 3b). Each profiling rung that introduces a code carries three explicit line items — registry row flip, doc page, and one observing test (check 5) — and those line items belong to the profiling plan's rung table, not to this ladder. The `W92xx` conditions themselves are raised inside `boyko_diag`/`profiling_abi` as sticky `DiagFlag`s and **emitted from `boyko_ecs`'s profiling fold**, because the substrate is diagnostically mute; this crate is the emitter of record for none of them. The full seam decision, including the narrowing of check 2 to `Live` rows and the strike of the duplicate invariant-TSC code, is `seam/diagnostic-code-space`.

---

## Decision 7: `Info`/`Debug`/`Trace` carry NO code; `Warn`/`Error` MUST

Different macro arities enforce it. A code is a promise of documentation, stability and an explanation; extending it to trace chatter makes the registry meaningless and check 2 unenforceable, and making it optional reproduces today's state — nine codes across thousands of diagnostics.

---

## Rate policy as a registry column *(Decision 8's registry half)*

The **mechanism** — the per-SITE `FIRED` latch that degrades to a pure `Relaxed` load from a private line, `ONCE_SITES`, the `LOG-ONCE` census rows, and the per-SITE-versus-per-CODE argument that settles the three-site `W2102` case — is `logging/emission-path`. What the **registry** owns, and what `codes!` must therefore generate, is three things:

1. **Every `W`/`E` row declares a `RatePolicy`**: `Every` / `Once` / `OnceCounted` / `EveryN(n)` / `MinIntervalMs(ms)`. The declaration is in the registry row, visible beside the code's summary, so the cost of a policy is legible where the code is defined rather than where it is emitted.
2. **`EveryN(n)` requires `n` to be a power of two** *(X3)*, enforced by `const _: () = assert!(n.is_power_of_two())` **inside `codes!`**, so the applied test is `count & (n-1)` instead of `count % n`. An arbitrary `n` mis-samples across the `u32` counter wrap (~12 h at 100 K·s⁻¹) — invisible in a 300-frame bench, wrong in a session. The fix is *also* cheaper: an `and` for a division.
3. **`code_idx` must stay dense**, because `RATE` is indexed by it. `Once` and `OnceCounted` do **not** use `RATE` at all (the latch is per site), so only `EveryN`/`MinIntervalMs` consume a slot's state — but the *index space* is shared with every downstream table, which is what Decision 19's minting protocol and its exhaustion behaviour exist to protect.

`RateSlot` is 64 B, one per cache line — four unrelated codes sharing a line (v1's 16 B slot) is false sharing between subsystems that have nothing to do with each other. `MAX_CODES = 512` ⇒ 32 KiB, in the same `.bss` regime as the lane array.

---

## Decision 19: Downstream code tables — the same macro, a different prefix, and a lazily-minted dense index

`codes!` is exported with a `prefix` parameter. A game invokes it once (`prefix = "acme"`, `doc_root = "docs/diag"`), gets its own `pub const` per code and its own `DiagInfo` table, and invokes `codes_tidy!(root = …, prefix = …)` to generate **the same eight checks over its own corpus** — because the engine's checks prove nothing about a game's registry, and that sentence is in G9's assertion message, not only in this document.

The `RATE` index must stay dense. Engine codes carry a compile-time `CodeIdx::Static(u16)`; downstream codes carry `CodeIdx::Dynamic(&'static AtomicU16)`, minted on first use with the reserve-then-publish protocol (`CAS UNASSIGNED→RESERVED`, `fetch_add` on `CODE_OCCUPANCY`, `store(Release)`), so 16 threads racing on one code produce exactly one index and leak none (G9). Cost on the downstream `Warn`/`Error` path only: one extra `Relaxed` load and one predicted-not-taken branch (~1 ns, measured by `downstream_code_warn` against the engine-code `warn!` in the same sitting). `CODE_OCCUPANCY` past 90 % emits `boyko-W0114`.

**What happens at 100 %, which v3 did not say** *(fixes M3)*. 512 slots are shared by the engine (whose own block map spans ~20 subsystems), every game table and every mod — and a modded title is the *expected* exhaustion case, on the game-facing path. The behaviour is defined, and the one thing it may never do is alias:

```rust
pub const CODE_IDX_EXHAUSTED: u16 = u16::MAX;   // a RESERVED sentinel, not an index
```

- The mint returns `CODE_IDX_EXHAUSTED`. It **never** wraps `fetch_add` into an occupied slot, because an aliased index silently applies **another code's** `EveryN`/`MinInterval` state — a rate policy secretly shared between two unrelated subsystems, which is worse than the loss it would be hiding.
- **The record is still delivered.** A `Warn`/`Error` is not lost because a table filled up; it is emitted with **`Every` semantics** (no rate policy applied), because the alternative is that the first symptom of exhaustion is silence.
- `boyko-E0115` fires **once**, naming the prefix and the code that could not be minted, and `LogStats.codes_unindexed` counts every subsequent unindexed emission. Both are printed by the census.

**G9 gains an exhaustion leg**: fill `CODE_OCCUPANCY` to `MAX_CODES`, mint once more ⇒ the returned index is the sentinel, `E0115` fires exactly once, the record still arrives, and no two codes share a `RateSlot`. **Red state**: make the mint `fetch_add(1) % MAX_CODES` ⇒ two codes resolve to one slot ⇒ the distinct-rate-state assertion fails.

**Decision 7 is NOT relaxed for games**: `Warn`/`Error` still MUST carry a code, and a code is still a promise of a documented page. Data-defined *codes* are refused (`logging/dispositions`) precisely because a data-defined code cannot have one.

### The registry's data structures

```rust
#[repr(C)] #[derive(Clone, Copy)] pub struct WarnCode  { num: u16, idx: CodeIdx }
#[repr(C)] #[derive(Clone, Copy)] pub struct ErrorCode { num: u16, idx: CodeIdx }
#[repr(C)] #[derive(Clone, Copy)] pub struct PanicCode { num: u16, idx: CodeIdx }
// Distinct newtypes ⇒ `warn!(T, codes::E2101, ..)` does not compile.

/// Engine codes carry a compile-time dense index; downstream codes carry a
/// pointer to a lazily-minted cell (Decision 19). Cost: one extra Relaxed load
/// and one predicted-not-taken branch, on the DOWNSTREAM Warn/Error path only.
#[derive(Clone, Copy)]
pub enum CodeIdx { Static(u16), Dynamic(&'static AtomicU16) }

#[repr(u8)] pub enum RatePolicy { Every, Once, OnceCounted, EveryN(u16), MinIntervalMs(u16) }
// `codes!` emits `const _: () = assert!(n.is_power_of_two())` for EveryN, so
// `count & (n-1)` is exact across a u32 wrap. `Once`/`OnceCounted` do NOT use
// `RATE` at all — the latch is per SITE (F11, `logging/emission-path`).

/// Registry ROW STATUS — the mechanism that lets L2 commit alone on a
/// grandfathered corpus (F20). `Pending` rows must have ZERO emitters (check
/// 3b); `Live` rows must have at least one (check 3) AND a doc page (check 2,
/// `Live`-only per S6). `Historical` (B6) is for a code that exists only in a
/// frozen artifact this repository will not edit: zero emitters, NO doc page
/// required, never becomes `Live`, excluded from check 3c's migration count.
#[derive(Clone, Copy, PartialEq, Eq)] pub enum CodeStatus { Live, Pending, Historical }

pub struct DiagInfo {
    pub number: u16, pub class: u8,
    pub prefix: &'static str,    // "boyko" for the engine; games declare their own
    pub summary: &'static str,   // one line, embedded, printable from a message
    pub rate: RatePolicy,
    pub status: CodeStatus,
    pub doc: &'static str,       // "docs/diagnostics/W1501.md" — check 2's target
}
static DIAGNOSTICS: [DiagInfo; N];       // dense, sorted; index == code_idx
const MAX_CODES: usize = 512;
static CODE_OCCUPANCY: AtomicU16;        // downstream minting; W0114 at 90 %
/// RESERVED sentinel, never an index. Returned when the 512-slot space is
/// exhausted; the record is still delivered, with `Every` semantics and no
/// rate state, and `boyko-E0115` fires once (M3). It is NEVER an aliased
/// index, because aliasing silently applies another code's EveryN/MinInterval
/// state to an unrelated subsystem.
pub const CODE_IDX_EXHAUSTED: u16 = u16::MAX;

/// 64 B — one code per cache line. `fired` is GONE (M1): it was dead from the
/// moment `Once`/`OnceCounted` stopped using `RATE` at all, and the census line
/// that read it printed a literal rather than an observation.
#[repr(C, align(64))]
struct RateSlot { count: AtomicU32, last_tsc: AtomicU64, suppressed: AtomicU32, _pad: [u8; 44] }
static RATE: [RateSlot; MAX_CODES];      // 32 KiB .bss

// ── the registry's macro surface ──────────────────────────────────────────────
/// Engine registry (single invocation, `prefix = "boyko"`), and the SAME macro
/// exported for downstream tables.
#[macro_export] macro_rules! codes { (prefix = $p:literal, doc_root = $d:literal, $($row:tt)*) => {...} }
/// Generates the EIGHT registry checks over a caller-supplied root+prefix.
/// The engine's own checks prove nothing about a downstream crate.
#[macro_export] macro_rules! codes_tidy { (root = $r:literal, prefix = $p:literal) => {...} }

pub fn explain(code: u16) -> Option<&'static DiagInfo>;
```

*(`ONCE_SITES` and the per-site `OnceSite` node are the emission path's structures and live in `logging/emission-path`; they are named here only because the census row they produce is what makes a `Once` policy's registry declaration observable.)*

---

## Integration — New

- **Crate `boyko_log`** — `level.rs`, `control.rs`, `target.rs`, `site.rs`, `record.rs`, `lane.rs` (`LogLane`), `codes.rs` (generated), `rate.rs` (+ `ONCE_SITES`), `sample.rs`, `sync_out.rs` (`OUT_LOCK`, `write_oracle_line` **and its durable fan-out**), `sink/{mod,console,file,binary,callback,crash,ecs,request}.rs` (`sink/ecs.rs` owns `ECS_HANDOFF`), `macros.rs`, `bin/logdec.rs`. **Deleted relative to v3**: `tsc.rs` (S4 — the clock is `boyko_diag`'s), `session.rs` (S11 — one `SessionId` mint, in `boyko_diag`), `build.rs` (S9 — the single axis is `BOYKO_PROFILE`, read by `crates/boyko_diag/build.rs`), and `report!` from `sync_out.rs` (S1).
- **Crate `boyko_diag`** — the shared substrate this plan consumes (clock, lane, loss, storage policy + `section_report`). **Specified in the `substrate/` half of this corpus**, not here, and jointly owned with the profiling half. It lands as rung **D0**, before L0.
- **`docs/diagnostics/<code>.md`** — one per `Live` code; check 2's target; published by `doc-writer`. **Shares a directory with this corpus** as of the split (see the corpus section): check 2 resolves an exact top-level filename.
- **`crates/boyko_ecs/src/ecs/core/log/`** — `LogPlugin`, `LogRing`, `LogStats`, `LogCensus`, `log_drain_system`.
- **`crates/boyko_macros`** — `#[derive(LogPod)]` (no new dependency edge: `boyko_macros` is a proc-macro crate with no `boyko_ecs` dependency).
- **`crates/boyko_log/tests/`** — `code_registry.rs` (the eight checks), `print_census.rs` (the tidy-style print ban, sharing the walker), `untested_codes.txt` and `print_allowlist.txt` (data files, each excluded from its own scan).
- **`docs/LOG-BINARY-FORMAT.md`** — the `.blog` schema with `schema_version`; the decoder refuses a mismatch.
- **`docs/HOT-PATH-EXCEPTIONS.md`** — **NO new entry.** See Invariant 1 (`logging/budgets-and-invariants`): a row for `OUT_LOCK` reds `scripts/check_hotpath_exceptions.py` because the file carries no `#[allow(clippy::disallowed_types)]` for it to match (F9).

---

## Migration ledger — machine-generated, not hand-tabled *(fixes M22)*

v1's table covered ~14 files against a measured **179 matching LINES across 36 files** under `crates/**/src/**` *(re-measured this session: **180 raw MATCHES** against those same **179 lines** in the same **36** files — one census, two grep conventions; see the note above)*. The migration is driven by a generated ledger, `docs/diagnostics/PRINT-CENSUS.md`, regenerated by the same walker that backs the enforcement test, with every site classified into exactly one of:

| Class | Count (measured) | Disposition |
|---|---|---|
| CLI binary stdout (`boyko_shaderdsl/src/bin/*`) | 58 *(re-confirmed)* | **Keep.** One crate-level `#![allow]` + rationale per bin, not per site. The only stdout writer in the workspace (S7). |
| Test-only files — **16 within-file + 7 cross-file** | 23 (`rhi_vulkan/compute/tests.rs` 16 within-file; `sdf_math/brick/tests.rs` 3 and `physics/solver/colored_tests.rs` 4 gated by a `#[cfg(test)]`-plus-`mod` declaration in the **parent**, the latter through a `#[path]`) — **all four counts re-confirmed** | **Keep.** Excluded by the walker's **cross-file** `#[cfg(test)]` rule (B7). v3's within-file rule would have missed 7 of them and driven test-only sites into the allowlist. |
| Measurement lines (`runner.rs`) | **0** *(was 20)* | **Not this plan's rows** *(S1)*. `report!` is deleted; the profiler migrates all six stdout consumers to its artifact at **profiling rung 7**, which lands **before** L8b. By the time L8b runs, these producers no longer exist. |
| Validation messenger (`debug.rs:114`) | 1 | **UNTOUCHED** (Decision 9b, F12 — `logging/sink-lifecycle`). Allowlisted with a reason, allowlist checked both ways. |
| Prose mentions inside comments (e.g. `runner.rs:560-561`) | ≥ 2 | **Not sites.** The walker's CODE stream never sees them, so they are not in the denominator (F18). |
| Everything else (production diagnostics) | the remainder | → `error!`/`warn!`/`info!` with codes. |

**The denominator, restated**: 179 raw occurrences − 58 (CLI) − 23 (test-only) = 98, minus the **20 measurement lines that S1 removes from this plan's scope** = **≤ 78** sites to disposition, of which the walker's CODE stream will resolve some number ≤ 78 as actual macro invocations. *(v2 said 83 — arithmetic error, F18; v3 said ≤ 98 — correct then, superseded by S1. The 180 figure quoted above is the same census under `grep -o` rather than a new site, and its extra match is a prose mention in a comment, which the row three lines up excludes from the denominator by rule — so the denominator stays **≤ 78** and does not move to 79. **The exit criterion is not this number** in any case, which is exactly why L8c is defined over the walker's own count.)* L8c's exit criterion is `Pending` == 0 in the registry (check 3c) plus zero unclassified walker sites — two integers, both machine-produced.

**Two dependency-hygiene items the migration owns** *(S12, S2)*:

- **`boyko_demo`'s third-party `log = "0.4"`** (`crates/boyko_demo/Cargo.toml:28`, used at `crates/boyko_demo/src/main.rs:113` — **re-verified this session**: `log::error!("boyko_demo failed to start: {err:?}")` on the wasm arm) is **deleted at L8b** and the site becomes `error!(Demo, codes::E3001, …)`. A tidy check then asserts **no workspace `Cargo.toml` names a third-party `log` or `tracing` dependency**; re-adding one reds it. *(`env_logger` at `:32` and `console_log` at `:69` — both re-verified — are wasm-console plumbing for that same dependency and go with it.)* The **wider** question — whether this is the whole story, given that `log 0.4` is in the build graph transitively via `eframe`/`egui`/`naga`/`gpu-allocator`/`bevy_ecs` regardless — is `substrate/tree-verification`'s open Q5, and the consequence it forces is that the tidy check can only ever be about **DIRECT declarations** and must say so in its own failure text.
- **`crates/boyko_image/Cargo.toml:5`'s description** — "no dependency on any other workspace crate" *(re-verified verbatim at `:5`)* — becomes **false** at L8a, when `boyko_image` gains `-> boyko_log` for `png.rs:206` / `inflate.rs:656`. The description is edited in the same commit; a stale description that contradicts the manifest below it is doc-rot with a two-line blast radius, and this plan is the one that creates it.

Named production files v1 omitted and that the ledger covers: `rhi_vulkan/present/targets.rs` (7), `render/texture.rs` (7), `app/{host_dump,hzb_dump,vg_census_dump,vb_probe_dump,vb_cull_probe}.rs` (14), `app/plugins.rs` (3), `app/gpu_scene/mod.rs` (3), `app/host.rs` (2), `physics/soft/self_collision.rs` (3), `ui/layout.rs` (2), `rhi/handle.rs` (2), `serialize/load.rs` (2), `ecs/asset/server.rs` (1), `ecs/ecs_master/system_api.rs` (1), `image/{png,inflate}.rs` (2), `render/{bindless,mesh_geometry_table,light_system,render_path_config,gpu_system}.rs` (8), `threadpool/worker.rs` (1), `ecs/schedule/schedule_builder.rs` (2).

---

## Behaviour changes worth naming

| Site | Change |
|---|---|
| `boyko_threadpool/src/worker.rs:159-168` *(v4 said `:157-168`; `fn abort_on_task_panic` opens at `:159`, its `eprintln!` is at `:163` and `std::process::abort()` at `:167`)* | `abort_on_task_panic` → `error!(codes::E0201, …)` + **`flush()` before `abort()`** |
| `boyko_ecs/.../schedule_builder.rs:1334-1350` | `warn_if_empty` → `warn!(Schedule, codes::W1501, …)`; text normalised (substring-safe) |
| `boyko_ecs/.../params/diagnostics.rs:53` | `error[boyko-B0002]:` → `boyko-B0002:` (substring-safe) |
| `boyko_ecs/.../events/event_buffer.rs` | overflow emits `warn!(codes::W0701, type_name, lane, attempted, dropped)`; the `Result` is unchanged. Those four fields currently exist only inside an `EcsError` nobody reads |
| `boyko_ecs/.../query_type_registry.rs:124-144` | `warn!(codes::W0501)` at 75 % occupancy; the terminal `panic!` gains `B0502`. 1023 silent mints then a process kill is not a diagnostic |
| `boyko_rhi_vulkan/src/debug.rs:114` | **NOT TOUCHED AT ALL** (Decision 9b, F12). v2's `to_string_lossy()` removal is withdrawn: `Cow::Borrowed` means there is no allocation to remove on the normal path, and writing raw `CStr` bytes would change the emitted bytes in the invalid-UTF-8 case on a byte-frozen gate-oracle channel pinned at `boyko_app/tests/vb_bench_query_validation.rs:118`. Added to `print_allowlist.txt` with that reason |
| `boyko_rhi_vulkan/src/device.rs:2110` *(L7a, **SHIPPED**, condition re-cut)* | `error!(codes::E2101)` when validation is requested and this process is **not getting it** — the escape hatch took it, or `VK_EXT_validation_features` is absent. **Not** "the node was not chained": the tree refutes that premise (F2), and the ladder's L7 block carries the measurement |
| `boyko_rhi_vulkan/src/device.rs:3100,3158,3189` *(L7b, **SHIPPED**; at HEAD the trio is inside `query_device_caps`, each `eprintln!` under a `#[cfg(debug_assertions)]` two lines above)* | drop `#[cfg(debug_assertions)]` → `warn!(codes::W2102)`, `RatePolicy::Once`. **Because `Once` is per-SITE (`logging/emission-path`, F11), all three degradations report** — a code-scoped `Once` would have printed one and silently lost two. Settles the two-doctrine conflict in favour of `boyko_app/src/host.rs:230-234`'s written argument that a release-build degrade-to-disabled must be observable *(v4 cited `:228-233`; the argument comment measures at `:230-234` and reads "Emitted UNCONDITIONALLY (not `#[cfg(debug_assertions)]`): a RELEASE-build degrade-to-Off must be observable")* |
| `boyko_rhi_vulkan/src/present/passes/gbuffer.rs` *(L7b, **SHIPPED**)* | hand-rolled `AtomicBool` latch deleted → `warn!(codes::W2104)` + `OnceSite`. **The latch was not what its own doc said**: the comment claimed "the one `swap` on the FIRST occurrence", the code did a `load` and a separate `store`, which excludes nothing — two callers arriving together both printed. `claim()` is one CAS |
| `boyko_rhi_vulkan/src/present/swapchain.rs` *(L7b, **SHIPPED**)* | present-mode fallback `eprintln!` → `warn!(codes::W2105)` + `Once`. `Once` because swapchain creation reruns per resize and the answer is a property of the surface, not the extent |
| `boyko_rhi_vulkan/src/present/targets.rs` ×7 *(L7b, **SHIPPED**)* | **two codes, not one** — `error!(codes::E2103)` ×4 for the rings `record_vb` `.expect()`s, `warn!(codes::W2106)` ×3 for the hwrt shadow chains it skips with `if let Some(..)`. `RatePolicy::Every` for both: they run at target build and each resize, never per frame. The failing ring's name is an **argument**, which is what L6-A's tagged payload bought — before it, seven distinguishable failures would all have printed one format literal |
| `boyko_render/src/render_path_config.rs:311-337` *(re-verified: `warned_ddgi.swap(true, Relaxed)` at `:311`, `warned_ssao.swap(true, Relaxed)` at `:335`, each inside an `#[inline]` per-frame reader)* | delete both hand-rolled latches (the per-frame `swap` bug) → `warn!` + `Once` |
| `boyko_render/src/light_system.rs:397,456` *(re-verified: `LOGGED.swap(true, Relaxed)` guards an `eprintln!` at each)* | delete latches → `warn!(codes::W2201, dropped_count)`; **the dropped count is now reported**, which the one-shot latch never did |
| `boyko_render/src/{bindless,mesh_geometry_table}.rs` | ad-hoc `"WARN: "` → `warn!(codes::W2202)`; keep `debug_assert!(false)`. Same per-site `Once` argument as `W2102` |
| `boyko_app` boot / teardown | **Cross-reference, not a row.** `boyko_app` owns the whole lifecycle (S5) — the boot and teardown order, `flush_gpu` ahead of `flush`, and the "nobody else may `boot`/`enable`/`shutdown`" rule are stated **once**, in `SEAM.md`. What is this ledger's is only that `boyko_app` selects a `LogRuntimePreset` and takes `LANE_HOST`, and that `boyko_demo` gains a console command bound to `apply_control_spec` as the worked example of runtime control |
| `boyko_threadpool` `set_lane` sites | **Cross-reference, not a row.** The write sites are `substrate/02-LANE.md`'s (`substrate/lane-write-sites`), which records **three**, not the two v4's ledger listed — `worker.rs:24`, `thread_pool.rs:190` and `thread_pool.rs:279` (`InstallGuard::drop`, the unwinding path). This is a `boyko_diag` rung (**D1**) and is named here only because it is the precondition for every lane index this crate uses |
| `boyko_demo/Cargo.toml:28` + `main.rs:113` | third-party `log = "0.4"` **deleted**; the one call site becomes an `error!` (S12) |
| `boyko_image/Cargo.toml:5` | description edited: it stops being true when the crate gains `-> boyko_log` |
| `boyko_render/src/gpu_system.rs:399-404` | → `error!(codes::E2203)`. The `System` trait's missing error channel stops mattering: the logger is a side channel available from any thread |
| `boyko_image/src/{png.rs:206, inflate.rs:656}` | → `warn!(codes::W2601/W2602)`; decoding continues |
| `boyko_app/src/runner.rs` (measurement sites) | **Not this plan's rows** (S1). Migrated to the profiler's artifact at profiling rung 7, before L8b. By L8b there is nothing measurement-shaped left in `runner.rs` for this ledger to disposition |

---

## Enforcement *(fixes M23)*

**Primary: an in-repo tidy-style test**, `crates/boyko_log/tests/print_census.rs`, which walks `crates/*/src/**.rs`, excludes `src/bin/` and `#[cfg(test)]` regions (by the **cross-file** rule above), asserts a non-empty corpus, and fails on any `println!`/`eprintln!`/`print!`/`eprint!` outside `tests/print_allowlist.txt` — with the allowlist checked in **both** directions. We own it, and it can be shown red in one line.

**Secondary: `clippy.toml`'s `disallowed-macros`**, added only after a **shown-red canary**. `clippy.toml:21-25` records, empirically, that clippy *silently ignores a config path it cannot resolve* — re-read verbatim this session: *"A path clippy cannot resolve is silently ignored (verified empirically 2026-07: an unresolvable entry emits nothing and does not suppress the resolvable ones)"*. The L8 gate compiles a deliberate `println!` and records the observed diagnostic in the plan's own gate log; if the key is inert on the pinned clippy, the entry is dropped and the tidy test stands alone. Independently noted: the lint cannot see `stdout().write_all`, `io::Write` on a raw handle, or `libc::write`, so it could never have carried the migration claim by itself.

---

## Compatibility

`Arena` / `ComponentPool` / `UnitId` untouched. `LogRing` and `LogCensus` use `VmReservation`-backed columns whose element sizes divide `COMMIT_GRANULE`, pinned by const asserts (M13, F7), **and whose `Send`/`Sync` is a manual impl with a named holder set** (B1) rather than a derivation `VmColumn` does not permit. `golden.ps1:226`'s `[vk-validation]` grep: preserved, still synchronous, **and its producer is not edited at all** (F12); `write_oracle_line` shares `stderr()`'s handle with it, so neither can splice a line into the other (S7).

**The `VB-P1d`/`VB-P4` parse contracts leave this plan entirely** *(S1)*. Nothing here writes stdout, so no golden and no parser moves **because of logging**. They do move at profiling rung 7, which owns that migration and its consequences — including the invalidation of every published floor number, since a floor measured on a different instrument bounds nothing. This document records the dependency (L8b lands after profiling rung 7) and makes no claim about the artifact channel's shape.
