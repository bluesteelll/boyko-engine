//! **`std::Vec` as a durable side store is the one shape that actually caused a data race here,
//! and it is the one shape nothing checks.**
//!
//! CLAUDE.md's principle 0 is explicit: durable per-entity / per-element / bulk subsystem data
//! lives in the ECS's own storage — `ComponentPool` columns, `Resource`-owned columns, dense
//! components — *"never `std::Vec` / `HashMap` as a side store"*. It cites its own precedent in
//! the same breath: *"A `std::Vec` physics mirror — a parallel data system glued on the side —
//! caused the O11-SP4 colored-solve data race; the fix is dense components in the kernel."*
//!
//! And the mechanical half of that rule does not cover it. [`clippy.toml`](../clippy.toml)'s
//! `disallowed-types` list — the thing that turns "forbidden on the hot path" into a build failure
//! — carries `HashMap`, `HashSet`, `Mutex`, `RwLock` and `Rc`. It does **not** carry `Vec`, so the
//! member of that sentence with a race behind its name is the member with no gate.
//!
//! A blanket ban would be wrong, which is presumably why nobody added one: a function-local
//! scratch `Vec` is legitimate, and CLAUDE.md names several more legitimate exceptions (the ECS's
//! own storage implementation, FFI / GPU / OS-contiguity buffers, lock-free threadpool internals).
//! The decidable form of the rule is not a type ban but a **field census**, and that is this file.
//!
//! # What is checked
//!
//! **A struct field whose declared type mentions `Vec<` — or a `static` item whose declared type
//! does — in any `.rs` under [`SCANNED_ROOT`], outside `#[cfg(test)]` regions and outside files
//! loaded only through a `#[cfg(test)]`-gated `mod` item, must appear on
//! [`KNOWN_VEC_FIELD_SITES`] — and each entry on that list must still be found, at exactly the
//! field count it declares. The entries sum to [`PINNED_VEC_FIELDS`], which is the pin the unified
//! plan reads (see "The pin" below).**
//!
//! Three shapes count as a field-bearing body, so the obvious ways around a braced-struct-only
//! reader are closed: a braced `struct`, a tuple `struct Foo(Vec<u32>);`, and a braced enum variant
//! `enum E { V { x: Vec<u32> } }`. The latter two carry no `Vec` in this crate today (measured —
//! the run prints the container count they contribute to), so they cost nothing now and exist
//! because a hole that costs nothing today is exactly the hole a future rung walks through.
//!
//! A **`static` item** is read wherever it is declared — at module level, inside a `thread_local!`
//! block (braced or written on one line), or inside a `fn` body — and with any type that spans
//! lines joined first. A static outlives every frame by construction, so the function-local
//! scratch argument below cannot cover it: `thread_local!` around a `Cell<Vec<_>>` is exactly the
//! "preallocate and reuse" shape principle 5 pushes an author toward, and it is a side store. A
//! static is reported as `NAME::<static>`; if one is ever excused, its row names the static as the
//! owner with a count of 1. It counts toward [`PINNED_VEC_FIELDS`] like a field. Measured on the
//! B4 trunk: six live statics (`row_identity.rs` 1, `profiling.rs` 2, `soft/self_collision.rs` 3),
//! none `Vec`-typed, so the reader costs no row and no pin change; the two `thread_local!`
//! statics in `narrowphase/box_box.rs` sit under `#[cfg(test)]` and are suppressed. The reader
//! was added by the B4 review fix — see "Review fix" below for the hole it closed.
//!
//! # What this predicate does NOT decide — stated here, and re-stated in the failure message
//!
//! * **It cannot tell a durable structure from a transient one.** That is the actual rule, and it
//!   is not decidable from a field declaration: `Vec<f32>` looks identical whether it is a
//!   frame-scratch buffer or a per-entity mirror that outlives the frame. So the gate forbids the
//!   **shape** and lets [`KNOWN_VEC_FIELD_SITES`] carry the **judgement**, one entry per site with
//!   a written argument — the same self-documenting discipline as the mandatory `// SAFETY:`
//!   comments and the `#[allow(clippy::disallowed_types)]` rationales.
//! * **It reads a struct FIELD and a `static` item, not every `Vec`.** A `Vec` in a function
//!   signature, a local, a return type, a `const`, a tuple type or a trait method is invisible to
//!   it, deliberately: a function-local scratch `Vec` is legitimate under CLAUDE.md and a gate that
//!   reported it would be a gate everyone learns to `#[allow]` past. One persistent form hides in
//!   that list: an ECS system parameter `Local<Vec<_>>` is state kept across runs, written in a
//!   function signature. `SCANNED_ROOT` holds **zero** `Local<` on the B4 trunk (measured, test
//!   files included). Whether a `Local` is kernel storage or a side store is a principle-0 question
//!   for the architect, not one a text census can settle, so the form is stated here, not decided.
//! * **It matches the TEXT `Vec<`, not a resolved type.** Two consequences, one in each direction.
//!   A field written through an alias — `type Rows = Vec<u32>;` then `rows: Rows` — is **invisible**
//!   and is the cheapest way around this gate; measured at `ad0ebea4` and again on the B4 trunk,
//!   `SCANNED_ROOT` contains **zero** `type … = …Vec<…>` aliases, so the hole is empty rather than
//!   merely unnoticed, but it is a hole and nothing here can close it without a type resolver. A
//!   use-rename is just as cheap — `use std::vec::Vec as Column;` then `rows: Column<u32>` — and so
//!   is a turbofish in type position, `rows: Vec::<u32>`, which rustfmt rewrites to `Vec<u32>` but
//!   which no gate runs rustfmt over. Both compile clippy-clean, and `SCANNED_ROOT` holds **zero**
//!   of either on the B4 trunk (`Vec as` 0, `Vec::<` 0, test files included). In the other
//!   direction, a field whose type merely *mentions* `Vec<` inside a generic argument
//!   (`Option<Vec<u32>>`, `[Vec<u32>; 4]`) **is** caught, which is intended — the storage is still
//!   a `Vec` on the side.
//!   `VecDeque<` contains no `Vec<` and is not matched; if one is ever added as a durable side
//!   store, this gate will not see it (zero `VecDeque` under `SCANNED_ROOT` on the B4 trunk).
//! * **A tuple ENUM variant is not read.** `enum E { V(Vec<u32>) }` slips through, where the
//!   struct form `struct Foo(Vec<u32>);` and the braced-variant form `enum E { V { x: Vec<u32> } }`
//!   do not. Zero enum variants in scanned code carry a `Vec` at `ad0ebea4` or on the B4 trunk
//!   (the trunk's one, `Rows(Vec<u32>)`, sits in `solver/colored_tests.rs`, a test-module file),
//!   and a tuple variant is a poor home for a durable column in any case, so this hole is stated
//!   rather than closed.
//! * **It reads lines, not tokens.** A field declaration inside a line-initial `/* … */` block
//!   comment would be read as live code. `SCANNED_ROOT` contains **zero** line-initial `/*`
//!   (measured at `ad0ebea4` and on the B4 trunk), the same trade
//!   [`ignore_reasons_census.rs`](ignore_reasons_census.rs) records for the same reason: a
//!   half-correct lexer wrong in an unpredicted way is worse than a line reader
//!   whose one blind spot is written down, and this one's failure mode is a *false red* naming a
//!   specific line.
//! * **`#[cfg(test)]` is skipped by attribute and brace region.** Any `#[cfg(…)]` whose predicate
//!   contains the whole token `test` — so `#[cfg(test)]`, `#[cfg(all(test, …))]` and
//!   `#[cfg(any(test, …))]` alike — suppresses everything up to the close of the block it
//!   introduces. At `ad0ebea4` the `all(test, …)` spelling had two live sites in
//!   `solver/colored.rs`; L11's `f8873aae` removed both, and the trunk's one live site is
//!   `#[cfg(all(test, not(miri)))]` in `broadphase_tree/kernel.rs` (`ecbfe416`). The run prints how
//!   many such regions it skipped, because a detector that stopped skipping would quietly inflate
//!   the field denominator instead of failing.
//! * **A test-module FILE is skipped by the `mod` item that declares it, not by its name.** A file
//!   that a parent pulls in with `#[cfg(test)] mod tests;` — or with `#[cfg(test)]
//!   #[path = "x_tests.rs"] mod tests;` — carries no cfg of its own, so the region skipper above
//!   cannot see it. It is skipped iff a top-level `mod NAME;` in a scanned file carries a test cfg
//!   in its attribute stack and resolves to it by Rust's lookup (a `#[path]` relative to the
//!   declaring file's directory; otherwise `NAME.rs` or `NAME/mod.rs` beside a `mod.rs` / `lib.rs`,
//!   and under `<stem>/` beside any other file). The run prints each skipped file next to the site
//!   that gates it. ⚠ Rung B4 changed this from a NAME rule (`*_tests.rs`, at `ad0ebea4`): the
//!   trunk gained a second layout — `broadphase_tree/tests.rs` under `#[cfg(test)] mod tests;`
//!   (`ecbfe416`) — which the name rule read as live code. A longer name list would also skip a
//!   live module that happened to carry one of its names. Every way the resolution can miss — a
//!   `mod` nested in an inline block, a `#[path]` through `..`, a blank line inside the attribute
//!   stack, an attribute on the `mod` item's own line, a child `mod` of a test file — leaves the
//!   file SCANNED: a false red naming a line, never a false green. A file that any top-level live
//!   `mod` item also resolves to stays scanned too, whatever test item also loads it: until the
//!   B4 review fix, `#[cfg(test)] #[path = "util.rs"] mod util_under_test;` beside a live
//!   `pub mod util;` skipped the live `util.rs` whole (zero such pairs on the B4 trunk — its two
//!   `#[path]` items each name a `*_tests.rs` file declared nowhere else). The one false-green
//!   shape left in this skip is the cfg predicate's, stated in the next bullet.
//! * ⚠ **And that predicate reads `#[cfg(not(test))]` as a test region too**, because it asks
//!   only whether the whole token `test` appears. A `Vec` field under `#[cfg(not(test))]` — live
//!   code by definition — would therefore be SKIPPED, which is a *false green*, the one direction
//!   this gate is not allowed to fail in, and the same predicate decides the test-module skip
//!   above, so `#[cfg(not(test))] mod x;` would skip `x.rs` whole. `SCANNED_ROOT` carries **zero**
//!   `#[cfg(not(test))]` (measured at `ad0ebea4` and on the B4 trunk), so the hole is empty rather
//!   than merely unnoticed; closing it needs the predicate to parse `not(...)` rather than scan
//!   for a token, which is the same half-correct
//!   lexer the line-reading trade above declines. Stated here so a future `#[cfg(not(test))]` in
//!   this crate is understood to open it.
//! * ⚠ **A field whose type is broken across lines at a comma is read only to the comma.**
//!   A declaration whose generic argument list is hand-wrapped, so that the line carrying the
//!   field name ends at a comma and the `Vec<` sits two lines below it, presents the parser with
//!   a first line that holds no `Vec<` — the field is classified from that line alone and the
//!   `Vec` is missed. Again a *false green*. Zero such declarations under `SCANNED_ROOT` at
//!   `ad0ebea4` or on the B4 trunk (measured: every field declaration's type closes on its own
//!   line), and rustfmt's default keeps them that way; a hand-wrapped generic would open it.
//!
//! # The exception list is EXACT, not a prefix
//!
//! [`KNOWN_VEC_FIELD_SITES`] names a struct **and the field count it is allowed**. A `Vec` added to
//! `SoftBody` reds even though `SoftBody` is on the list; a `Vec` deleted from `IslandSleep` also
//! reds, because a list entry that over-states its subject reads as coverage it no longer has —
//! the `gone`-clause defect `gpu_blocking_reader_census.rs` records, where a pinned row outlived
//! its subject by seven commits. That two-sided exactness is the half that makes this more than a
//! snapshot of today's tree.
//!
//! Both entries are **named rungs**, not defects to repair here: at `ad0ebea4` the island-sleep
//! migration and soft-body rung S0; on the trunk, physics rungs U6 and S0 of
//! `docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md` §12, scheduled in the unified plan's Phase E.
//!
//! # The pin: 30 since L10 C0 — 34 re-derived on the trunk (rung B4), delta 0, until then
//!
//! **L10 C0 (2026-09-24) migrated `IslandSleep`'s four `Vec` fields onto two kernel columns**
//! (`docs/physics/perf-campaign/levers/L10-sleeping/04-DESIGN-REV2.md` D14: the per-row
//! `asleep` / `below_count` joined the island contact key in one `SleepLatch` column, and the
//! per-island `frozen_islands` / `energy` became one `IslandScratch` column). That pre-empts
//! U6's `IslandSleep` removal, so its row left [`KNOWN_VEC_FIELD_SITES`] and
//! [`PINNED_VEC_FIELDS`] went 34 → 30 in the same commit, by the rule below. The receipts in
//! this header that name `IslandSleep` (m2, m3, m3b, b2) are the gate's red-first history on
//! the tree that still held it.
//!
//! [`PINNED_VEC_FIELDS`] is the number the unified plan reads
//! (`docs/unification/UNIFIED-SYSTEM-PLAN-02-ORDER-OF-WORK.md`, Phase B row B4 and the Phase E
//! note "U6's and S0's census numbers are read from B4's pin"): U6 lowers it by `IslandSleep`'s 4
//! (34 → 30) and S0 by `SoftBody`'s 30 (30 → 0). A compile-time assertion holds it equal to the
//! sum of the [`KNOWN_VEC_FIELD_SITES`] counts, so a row lowered without the pin, or the pin
//! without a row, does not build. S0, which reaches 0, must also retire anti-vacuity guard 2
//! deliberately, because that guard reds on an empty find.
//!
//! This file landed in `ad0ebea4` on `feat/ecs-native-storage`, which is **not** an ancestor of
//! the trunk. Its scanned tree is its parent `05fcbd1d`'s, the merge base, because `ad0ebea4`
//! adds only this file. B4 ran the file unchanged on the trunk (`integ/unified` @ `c1e9f1db`) and
//! compared both trees' findings row by row, keyed by (file, struct, field, declared type):
//!
//! | findings | `ad0ebea4` | trunk | attribution |
//! |---|---|---|---|
//! | `resources.rs::IslandSleep` — `asleep`, `below_count`, `frozen_islands`, `energy` | 4 | 4 | same names and types. The block moved 140 lines down (`asleep` :3069 → :3209) under edits above it: KE16 `67563d3b` (−12), `b74f7ee8` (+16, one of them inside the struct doc), `a56007ab` (+23), `56c1e9e7` (+11), `00c07d0f` (+9), `caac7d06` (+6), L5 `b8d9ab8f` (+60) and `de06b6c9` (+6), the tree broadphase `ecbfe416` (+20), L11 `691891c4` (+1). Lines are not pinned |
//! | `soft/component.rs::SoftBody` — 30 fields | 30 | 30 | the file is untouched since the merge base; same lines |
//! | `broadphase_tree/tests.rs::Sim` — `bodies`, `ids`, `fresh` | — | 3 | `ecbfe416`, the tree broadphase (merged by `bbd5d12c`) |
//! | `broadphase_tree/tests.rs::Sim` — `generation` | — | 1 | `c52ad183`, the harness keyed by slot and generation so it builds against A1b's `RowIdentity` |
//!
//! **The four new findings are not side stores.** `Sim` is a test harness in a file that
//! `broadphase_tree/mod.rs` declares as `#[cfg(test)] mod tests;` — a test-module layout the name
//! rule did not know. So B4 adapted the scanner, not the list (the test-module bullet above), and
//! every other finding is unchanged: **pin 34 on the trunk, delta 0, no row added, none removed.**
//! The lanes merged since the merge base added live modules — `row_identity.rs` (`b74f7ee8`, then
//! A1b's `8af0e3b9`), `body_set.rs` (`a56007ab`), `profiling.rs` (`13748cec`),
//! `narrowphase/dispatch.rs` (L5, `b8d9ab8f`), `solver/warm_records.rs` (L11, `691891c4`) and
//! `broadphase_tree/{mod,bvh,kernel}.rs` (`ecbfe416`). None of them declares a live `Vec` field:
//! the only ones in them, 7 in `row_identity.rs` and 7 in `narrowphase/dispatch.rs`, sit in
//! `#[cfg(test)] mod tests` blocks, as do the 3 that A7b (`8d656ad8`) added to
//! `narrowphase/box_box.rs`.
//! The denominators grew from 30 files, 62 containers and 377 fields (17 test regions and 2
//! test-module files skipped) to 38 files, 97 containers and 500 fields (49 and 3). Run unchanged,
//! `ad0ebea4`'s scanner read 39 files, 101 containers, 518 fields and 38 `Vec` fields on the
//! trunk: the difference is exactly `broadphase_tree/tests.rs`. Run on `ad0ebea4`'s tree, this
//! adapted scanner reproduces 30 / 62 / 377 / 34 exactly, so the adaptation changes nothing but
//! the new layout's file.
//!
//! # Anti-vacuity — three guards, three messages
//!
//! A census that scans nothing reports a triumphant zero, which is this corpus's most-repeated
//! defect class. So:
//!
//! 1. **[`MIN_CONTAINERS`] / [`MIN_FIELDS`] / [`MIN_STATICS`] floors** — a walk that stopped
//!    walking, a field parser that stopped parsing, or a `static` reader that stopped reading,
//!    reds instead of passing over an empty set. *Fired by m5 below, and by r2 for the statics.*
//!    The statics floor has to be its own clause: zero statics are `Vec`-typed today, so guard 2
//!    cannot notice a dead static reader either.
//! 2. **At least one `Vec` field FOUND** — a type matcher that stopped matching would otherwise
//!    report "no side stores" over a crate that has 34 of them, and it would do so with the field
//!    denominator **unchanged**, which is why guard 1 cannot stand in for it. *Fired by m4.*
//! 3. **Every [`KNOWN_VEC_FIELD_SITES`] entry PRESENT** — deleting the code the gate points at is a
//!    red, not a silent pass. *Fired by m3b.*
//!
//! Every count the gate enforced is printed, the way the sibling censuses print theirs, so the
//! figures in this comment are re-derivable from a run rather than remembered:
//! `cargo test -p boyko-engine --test physics_vec_side_store_census -- --nocapture`.
//!
//! # Red-first: the receipts at `ad0ebea4`
//!
//! Line numbers and denominators in this section are `ad0ebea4`'s tree; the trunk's receipts
//! follow in the next section.
//!
//! Five mutations, each applied to the shipped tree and then restored by `cp` from a `cmp`-proved
//! snapshot (both files verified byte-identical, and `resources.rs` by md5, after the last one).
//! The outputs are pasted so a future reader can tell a working gate from one that has quietly
//! stopped firing; the standing narrowing block every message carries is elided here with `…`.
//!
//! ⚠ One more edit to the pasted text, made by rung B4, here and in the next section: the census
//! prints a site as the file, a colon and the line; these receipts write it as `file @ line N`.
//! [`internal_docs_anchors.rs`](internal_docs_anchors.rs) reads a colon-and-number after a `.rs`
//! name in any Rust source as a claim about the CURRENT tree, and every such line here is a claim
//! about `ad0ebea4`'s tree or about a mutated one. In the colon form they bound as live citations
//! and moved that gate's own pinned counts.
//!
//! **(m1) A `Vec<u32>` field added to a struct that has none** — `contact_count: Vec<u32>,`
//! inserted into `ContactPairs` (`resources.rs`), which is exactly the regression this gate exists
//! for: that struct's one real column is a migrated `ScratchColumn`.
//!
//! ```text
//! test no_unlisted_vec_side_store_in_boyko_physics ... FAILED
//! …62 field-bearing container(s), 378 field declaration(s) parsed, 35 of them `Vec`-typed…
//! a NEW `std::Vec` side store has appeared in boyko_physics:
//!   crates/boyko_physics/src/resources.rs @ line 547  `ContactPairs::contact_count: Vec<u32>`
//! ```
//!
//! **(m2) A second `Vec` field added to `IslandSleep`** — on the exception list, and still red,
//! because the row names a count and not a prefix:
//!
//! ```text
//! test every_known_vec_side_store_is_present_at_its_declared_count ... FAILED
//!   crates/boyko_physics/src/resources.rs::IslandSleep — 5 found, 4 allowed
//! crates/boyko_physics/src/resources.rs::IslandSleep has GAINED 1 `Vec` field(s): 5 found, 4 allowed.
//!   …
//!   crates/boyko_physics/src/resources.rs @ line 3087  `island_scratch: Vec<u32>`
//! ```
//!
//! **(m3) A `Vec` field deleted from `IslandSleep`** (`energy: Vec<f32>` → `energy: f32`) — the
//! stale-exception direction, which is the shape that licenses rot:
//!
//! ```text
//! test every_known_vec_side_store_is_present_at_its_declared_count ... FAILED
//!   crates/boyko_physics/src/resources.rs::IslandSleep — 3 found, 4 allowed
//! crates/boyko_physics/src/resources.rs::IslandSleep has SHED 1 `Vec` field(s): 3 found, 4 allowed.
//! ```
//!
//! Two further mutations were run because a guard whose red nobody has seen is a guard nobody
//! knows the shape of, and m1–m3 between them fire only *two* of the three anti-vacuity clauses.
//!
//! **(m3b) `IslandSleep` renamed away** — guard 3 proper, the branch m3 does not reach because
//! three of the four fields survive it:
//!
//! ```text
//!   crates/boyko_physics/src/resources.rs::IslandSleep — 0 found, 4 allowed
//! crates/boyko_physics/src/resources.rs::IslandSleep — the gate found NO `Vec` field there at all (expected 4).
//! ```
//!
//! **(m4) The type matcher broken** ([`VEC_NEEDLE`] `"Vec<"` → `"Vec8<"`) — guard 2. Note that the
//! field denominator is *unchanged* at 377, which is precisely why a floor on fields scanned
//! cannot substitute for this clause:
//!
//! ```text
//! …62 field-bearing container(s), 377 field declaration(s) parsed, 0 of them `Vec`-typed…
//! the scan found ZERO `Vec`-typed fields under crates/boyko_physics/src. That is not a clean
//! tree: 2 exception row(s) name sites that are supposed to be found. …
//! ```
//!
//! **(m5) The walk broken** ([`SCANNED_ROOT`] pointed at `…/src/narrowphase`) — guard 1:
//!
//! ```text
//! the scan of crates/boyko_physics/src/narrowphase entered only 9 field-bearing container(s)
//! across 4 file(s) (floor 35). The walker or the struct detector is broken, not the tree …
//! ```
//!
//! # Red-first on the trunk (rung B4)
//!
//! Each mutation was applied by editing one line, then reverted by editing it back; every file
//! was `cmp`-identical to its pre-mutation snapshot afterwards and `git diff -- crates/` was clean.
//! Baseline: 38 files, 97 containers, 500 fields, 34 `Vec`-typed, 5 of 5 tests passed.
//!
//! **(b1) A `Vec` field added to a live struct a merged lane brought** — `probe_rows: Vec<u32>,` as
//! the first field of `BroadphaseTree` (`broadphase_tree/mod.rs`, `ecbfe416`):
//!
//! ```text
//! …97 field-bearing container(s), 501 field declaration(s) parsed, 35 of them `Vec`-typed (pin 34)…
//! test no_unlisted_vec_side_store_in_boyko_physics ... FAILED
//! a NEW `std::Vec` side store has appeared in boyko_physics:
//!   crates/boyko_physics/src/broadphase_tree/mod.rs @ line 251  `BroadphaseTree::probe_rows: Vec<u32>`
//! ```
//!
//! **(b2) A `Vec` field removed** (`IslandSleep::energy: Vec<f32>` → `f32`). It is red in both
//! tests, and it stays red until the pin is lowered:
//!
//! ```text
//! crates/boyko_physics/src/resources.rs::IslandSleep has SHED 1 `Vec` field(s): 3 found, 4 allowed.
//! the scan found 33 `Vec`-typed field(s) under crates/boyko_physics/src; the pin is 34. …
//! test result: FAILED. 3 passed; 2 failed
//! ```
//!
//! Lowering the row alone (`fields: 4` → `3`) does not build:
//!
//! ```text
//! error[E0080]: evaluation panicked: PINNED_VEC_FIELDS must equal the sum of the KNOWN_VEC_FIELD_SITES field counts
//! ```
//!
//! Lowering the row and [`PINNED_VEC_FIELDS`] together (34 → 33) is green: the run reads 33
//! `Vec`-typed fields against pin 33, and 5 of 5 pass. That is the edit U6 makes, with −4.
//!
//! **(b3) The test-module skip cannot hide a live file** — `#[cfg(test)]` deleted from
//! `broadphase_tree/mod.rs`'s `mod tests;`, so the same file under the same name is now live
//! code:
//!
//! ```text
//! …39 file(s) scanned, 2 `#[cfg(test)]` module file(s) skipped…101 field-bearing container(s),
//! 518 field declaration(s) parsed, 38 of them `Vec`-typed (pin 34)…
//! a NEW `std::Vec` side store has appeared in boyko_physics:
//!   crates/boyko_physics/src/broadphase_tree/tests.rs @ line 67  `Sim::bodies: Vec<BodyState>`
//!   crates/boyko_physics/src/broadphase_tree/tests.rs @ line 68  `Sim::ids: Vec<usize>`
//!   crates/boyko_physics/src/broadphase_tree/tests.rs @ line 73  `Sim::generation: Vec<u32>`
//!   crates/boyko_physics/src/broadphase_tree/tests.rs @ line 75  `Sim::fresh: Vec<usize>`
//! ```
//!
//! # Review fix: `static` items, and a live file a test item also loads
//!
//! The B4 review found the census blind to a `Vec` held in a `static` or a `thread_local!` (W1),
//! a form this crate already writes for non-`Vec` state. The fix added the `static` item reader,
//! the [`MIN_STATICS`] floor and the positive cases in both synthetic controls, and it keeps a file
//! scanned when a live `mod` item also loads it (O2). Pin, rows and denominators are unchanged:
//! 38 files, 97 containers, 500 fields, 6 `static` items, 34 `Vec`-typed, 5 of 5 tests passed.
//! The receipts above predate the reader, so their report lines carry no `static` count. Each
//! mutation below was reverted and `cmp`-verified against its snapshot.
//!
//! **(r0) The hole, on the scanner as B4 first committed it** — three lines inserted after
//! `ROW_IDENTITY_EPOCH` in `row_identity.rs`: a `pub static` `OnceLock<Vec<u32>>`, a one-line
//! `thread_local! { static …: Cell<Vec<u32>> = … }`, and a static whose `OnceLock<` type wraps
//! onto a `Vec<u8>` on the next line. All three compile. The old scanner stayed green:
//!
//! ```text
//! …97 field-bearing container(s), 500 field declaration(s) parsed, 34 of them `Vec`-typed (pin 34)…
//! test result: ok. 5 passed; 0 failed
//! ```
//!
//! **(r1) The same three lines, with the fix:**
//!
//! ```text
//! …500 field declaration(s) and 9 `static` item(s) parsed, 37 of them `Vec`-typed (pin 34)…
//! a NEW `std::Vec` side store has appeared in boyko_physics:
//!   crates/boyko_physics/src/row_identity.rs @ line 118  `B4_FIX_ROWS::<static>: std::sync::OnceLock<Vec<u32>>`
//!   crates/boyko_physics/src/row_identity.rs @ line 119  `B4_FIX_SCRATCH::<static>: std::cell::Cell<Vec<u32>>`
//!   crates/boyko_physics/src/row_identity.rs @ line 120  `B4_FIX_WRAPPED::<static>: std::sync::OnceLock< Vec<u8>, >`
//! test result: FAILED. 4 passed; 1 failed
//! ```
//!
//! **(r2) The static reader broken** (`static_items`'s keyword `"static"` → `"statik"`). The
//! `Vec` count stays at 34, because no live static is `Vec`-typed, so guard 2 cannot see this. The
//! statics floor and both synthetic controls do:
//!
//! ```text
//! …500 field declaration(s) and 0 `static` item(s) parsed, 34 of them `Vec`-typed (pin 34)…
//! the scan read only 0 `static` item(s) (floor 3). The static-item reader has stopped reading. …
//! static_items("pub static ROWS: std::sync::OnceLock<Vec<u32>> = std::sync::OnceLock::new();") read the wrong thing
//!   left: []
//! test result: FAILED. 2 passed; 3 failed
//! ```
//!
//! **(r3) The join for a type that wraps across lines disabled** ([`STATIC_JOIN_LOOKAHEAD`]
//! `8` → `0`). The container control loses `WRAPPED::<static>`, and red-first run r1 shows the same
//! shape on the live tree (`B4_FIX_WRAPPED`):
//!
//! ```text
//!   left: ["Braced::kept", …, "ROWS::<static>", "SCRATCH::<static>", "LOCAL::<static>"]
//!  right: ["Braced::kept", …, "ROWS::<static>", "SCRATCH::<static>", "WRAPPED::<static>", "LOCAL::<static>"]
//! test result: FAILED. 4 passed; 1 failed
//! ```
//!
//! **(r4) The O2 exclusion removed** (the `retain` that drops a file a live `mod` item also
//! resolves to). The module-file control then skips the live `util.rs`:
//!
//! ```text
//!   left: [… ("c/src/t.rs", "c/src/lib.rs @ line 5"), ("c/src/util.rs", "c/src/other.rs @ line 3")]
//!  right: [… ("c/src/t.rs", "c/src/lib.rs @ line 5")]
//! test result: FAILED. 4 passed; 1 failed
//! ```
//!
//! # Scope, and the widening this file owes
//!
//! **This gate covers `boyko_physics` only**, and that is a narrower claim than the rule it
//! enforces. The workspace-wide figure, measured with
//!
//! ```text
//! grep -rnE '^\s*(pub(\([a-z_]+\))?\s+)?[a-z_][A-Za-z0-9_]*\s*:\s*.*Vec<' crates/*/src \
//!   --include=*.rs | grep -v _tests.rs | wc -l
//! ```
//!
//! was **332 field-shaped lines across 20 crates** at `ad0ebea4` (`boyko_ecs` 77, `boyko_ui` 45,
//! `boyko_physics` 34, `aether_lang` 30, `boyko_rhi_vulkan` 29, …) and is **370 across 20** on the
//! B4 trunk (`boyko_ecs` 77, `boyko_physics` 56, `boyko_ui` 47, `boyko_rhi_vulkan` 33,
//! `boyko_render` 31, `aether_lang` 30, …). **Most of those are legitimate**: the ECS's own
//! storage implementation is an explicit CLAUDE.md exception, and compilers, macro expanders,
//! builders and boot plumbing are not engine paths at all. That grep is also coarser than this
//! file's scanner — it does not exclude `#[cfg(test)]` modules and does not join multi-line
//! declarations — so treat it as an order of magnitude, not a census: of the trunk's 56
//! `boyko_physics` lines, this scanner counts 34, and the other 22 are test code plus one
//! function parameter.
//!
//! So a workspace-wide form is a **separate deliverable with real remediation behind it**, not a
//! constant change — and until it lands, this paragraph is the record that the scope below is
//! narrower than the rule, in the shape [`gaia_g0_citation_census.rs`](gaia_g0_citation_census.rs)
//! established for its own owed widening. The scanned root is a single named constant
//! ([`SCANNED_ROOT`]) precisely so that the widening, when it is called, is one line here plus the
//! exception rows the remediation leaves behind.
//!
//! # Home
//!
//! The workspace-root package (`boyko-engine`), hand-rolled scanning, zero dependencies — the two
//! reasons [`internal_docs_anchors.rs`](internal_docs_anchors.rs) records, verbatim:
//! `CARGO_MANIFEST_DIR` **is** the repository root, so no `../..` walking can silently aim the scan
//! at the wrong tree; and that package has no dependencies, so the gate needs no engine build and
//! no GPU. Hand-rolled for the same reason — there is no regex crate to reach for.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The one directory this gate scans, repo-relative and `/`-separated.
///
/// Widening the gate is a change to this constant plus whatever exception rows the remediation
/// leaves behind. See the module doc's scope section for what that would cost.
const SCANNED_ROOT: &str = "crates/boyko_physics/src";

/// A `Vec`-typed struct field the gate permits, named exactly.
///
/// `fields` is the count of `Vec`-mentioning fields the named struct is allowed — not a minimum
/// and not a prefix. One more reds, one fewer reds.
struct Allowed {
    /// Repo-relative, `/`-separated, as the scanner reports it.
    file: &'static str,
    /// The struct's declared name.
    owner: &'static str,
    /// Exactly how many `Vec`-mentioning fields it may declare.
    fields: usize,
    /// The rung that removes this entry. A row without one is a parking space, not an exception.
    rung: &'static str,
}

/// The durable `Vec` side store that survives in `boyko_physics`: one since L10 C0 migrated
/// `IslandSleep` (design D14), two at `ad0ebea4` and on the B4 trunk.
///
/// It is a **named rung**, which is why it is an exception rather than a repair. Everything else
/// in the crate has already migrated: 202 `ScratchColumn` lines at `ad0ebea4` (across `lib.rs`,
/// `narrowphase/axis_cache.rs`, `resources.rs`, `scratch_ids.rs`,
/// `solver/{colored,soft_step,warm_start}.rs` and `soft/{colored,coupling}.rs`), and 235 on the B4
/// trunk, where the merged lanes' `broadphase_tree/{mod,bvh}.rs`, `narrowphase/dispatch.rs`,
/// `row_identity.rs` and `solver/warm_records.rs` join the list. Both figures come from the same
/// `grep -rn ScratchColumn` line count, test files included.
///
/// ⚠️ The counts here are the gate's whole point and are **derived from a run**, not copied from a
/// brief. The prescription that ordered this gate said "~38 `Vec`-typed struct fields … `SoftBody`
/// (~26 fields)"; the scanner in this file measures **34 and 30**, cross-checked against an
/// independent grep that agrees on both the total and the per-file split. A gate seeded from a
/// remembered number is a gate that reds on the day it is installed, or worse, passes a count
/// nobody ever held.
const KNOWN_VEC_FIELD_SITES: &[Allowed] = &[
    Allowed {
        file: "crates/boyko_physics/src/soft/component.rs",
        owner: "SoftBody",
        fields: 30,
        rung: "physics rung S0, docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md §12 (pin 30 → 0), \
               rung S0 of the soft-body ladder in docs/physics/ADVANCED-PHYSICS-DESIGN-SPACE.md — \
               the ten particle columns, four constraint columns, six tet columns, seven coupling \
               columns and three self-collision columns become dense components",
    },
];

/// The pin: exactly how many `Vec`-typed fields the scan may find.
///
/// 34 at `ad0ebea4` and 34 on the B4 trunk, delta 0; 30 since L10 C0, which migrated
/// `IslandSleep`'s four fields ahead of U6 (module doc, "The pin"). The unified plan reads its
/// physics census numbers from this constant: S0 takes it to 0. The assertion
/// below holds it equal to the sum of [`KNOWN_VEC_FIELD_SITES`]' counts, so the pin and the rows
/// can only move together — and they move only in the commit that migrates a store, never to
/// meet green.
const PINNED_VEC_FIELDS: usize = 30;

const _: () = {
    let mut sum = 0;
    let mut i = 0;
    while i < KNOWN_VEC_FIELD_SITES.len() {
        sum += KNOWN_VEC_FIELD_SITES[i].fields;
        i += 1;
    }
    assert!(
        sum == PINNED_VEC_FIELDS,
        "PINNED_VEC_FIELDS must equal the sum of the KNOWN_VEC_FIELD_SITES field counts"
    );
};

/// The text a declared type must mention to be a finding.
///
/// A named constant rather than five inline literals, so the "type matcher stopped matching"
/// failure — the one anti-vacuity guard 2 exists for — is reproducible as a ONE-LINE mutation.
/// A guard whose red has never been seen is a guard nobody knows the shape of.
const VEC_NEEDLE: &str = "Vec<";

/// Floor on field-bearing containers walked. The crate holds far more; this catches a walker that
/// stopped walking, not a tree that changed.
const MIN_CONTAINERS: usize = 35;

/// Floor on field declarations parsed. Same purpose: a field parser that silently stopped parsing
/// would otherwise report a clean tree.
const MIN_FIELDS: usize = 200;

/// Floor on `static` items read. Six live ones on the B4 trunk; half of that catches a static
/// reader that stopped reading, which neither the field floors nor guard 2 can see while no static
/// is `Vec`-typed.
const MIN_STATICS: usize = 3;

/// Cap on how many lines one field declaration may span before the join gives up. The longest real
/// one is two; the cap keeps a malformed file from running the join to the end of the body.
const FIELD_JOIN_LOOKAHEAD: usize = 8;

/// Cap on how many lines a tuple-struct declaration may span before its `;` is expected.
const TUPLE_JOIN_LOOKAHEAD: usize = 8;

/// Cap on how many lines a `static` item's declared type may span before its `=` or `;`. Reaching
/// the cap keeps everything joined so far as the type — a false red at worst, never a missed `Vec`.
const STATIC_JOIN_LOOKAHEAD: usize = 8;

/// One `Vec`-typed field, as the failure message reports it.
struct VecField {
    /// Repo-relative, `/`-separated.
    file: String,
    /// 1-indexed line of the declaration's LAST line (the one carrying the `,`), which is where a
    /// reader's eye lands on the type.
    line: usize,
    /// The struct — or `Enum::Variant` — that declares it, or the `static` item's own name.
    owner: String,
    /// The field's name, `<positional>` for a tuple struct, or `<static>` for a `static` item.
    field: String,
    /// The declared type, as written.
    ty: String,
}

/// Everything one scan of [`SCANNED_ROOT`] yields.
struct Census {
    /// `.rs` files read.
    files: usize,
    /// Test-module files skipped, each as `file <- site of the gating mod item`, listed so the
    /// exclusion is visible rather than assumed.
    skipped_test_files: Vec<String>,
    /// `#[cfg(test)]`-gated brace regions suppressed.
    cfg_test_regions: usize,
    /// Field-bearing bodies entered: braced structs, tuple structs, braced enum variants.
    containers: usize,
    /// Field declarations parsed, of any type.
    fields: usize,
    /// `static` items read, of any type.
    statics: usize,
    /// The fields and `static` items whose declared type mentions `Vec<`.
    vec_fields: Vec<VecField>,
}

/// The workspace root. This test lives in the root package, so the manifest dir *is* the root.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// True for the characters that make a token part of a longer identifier.
fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Does `ident` occur in `text` as a whole token?
fn contains_token(text: &str, ident: &str) -> bool {
    let bytes = text.as_bytes();
    text.match_indices(ident).any(|(idx, _)| {
        let before_ok = idx == 0 || !is_ident_char(bytes[idx - 1] as char);
        let after = idx + ident.len();
        let after_ok = after >= bytes.len() || !is_ident_char(bytes[after] as char);
        before_ok && after_ok
    })
}

/// Drop the `//` line comment, if any, respecting string literals.
///
/// A doc comment collapses to whitespace, which is what keeps `/// the `Vec<f32>` columns` — the
/// shape every field in `SoftBody` is documented with — from being read as a declaration.
fn strip_line_comment(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_str = false;
    let mut escaped = false;
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if in_str {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
            }
            out.push(c);
        } else if c == '"' {
            in_str = true;
            out.push(c);
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            break;
        } else {
            out.push(c);
        }
        i += 1;
    }
    out
}

/// Replace every string literal's body with nothing, so a token search cannot match inside one.
fn strip_strings(code: &str) -> String {
    let mut out = String::with_capacity(code.len());
    let mut in_str = false;
    let mut escaped = false;
    for c in code.chars() {
        if in_str {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
                out.push('"');
            }
            continue;
        }
        if c == '"' {
            in_str = true;
            out.push('"');
        } else {
            out.push(c);
        }
    }
    out
}

/// Net brace depth change of one line, counting only braces OUTSIDE string literals.
fn brace_delta(code: &str) -> i32 {
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escaped = false;
    for c in code.chars() {
        if in_str {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {}
        }
    }
    depth
}

/// Is this line a `#[cfg(…)]` attribute whose predicate mentions the whole token `test`?
///
/// Covers `#[cfg(test)]`, `#[cfg(all(test, …))]` and `#[cfg(any(test, …))]` alike. String literals
/// are stripped first so a hypothetical `#[cfg(feature = "test_only")]` cannot be read as one.
fn is_cfg_test_attr(trimmed: &str) -> bool {
    trimmed.starts_with("#[cfg(") && contains_token(&strip_strings(trimmed), "test")
}

/// The name declared by `<qualifiers> struct NAME` / `<qualifiers> enum NAME` on this line, plus
/// whatever follows it, or `None` when the line declares neither.
///
/// Returns `(name, rest)` where `rest` begins at the first character after the name — which is
/// what separates a tuple struct (`rest` opens `(`, possibly after generics) from a braced one.
fn declared<'a>(code: &'a str, keyword: &str) -> Option<(String, &'a str)> {
    let mut from = 0usize;
    let bytes = code.as_bytes();
    loop {
        let offset = code[from..].find(keyword)?;
        let start = from + offset;
        let end = start + keyword.len();
        let before_ok = start == 0 || !is_ident_char(bytes[start - 1] as char);
        let after_ok = code[end..].chars().next().is_none_or(|c| !is_ident_char(c));
        if before_ok && after_ok {
            let rest = code[end..].trim_start();
            let name: String = rest.chars().take_while(|c| is_ident_char(*c)).collect();
            if name.is_empty() || !name.starts_with(|c: char| c.is_alphabetic() || c == '_') {
                return None;
            }
            let tail = &rest[name.len()..];
            return Some((name, tail));
        }
        from = end;
    }
}

/// Skip a balanced `<…>` generic parameter list at the head of `tail`, if present.
fn skip_generics(tail: &str) -> &str {
    let t = tail.trim_start();
    if !t.starts_with('<') {
        return t;
    }
    let mut depth = 0i32;
    for (idx, c) in t.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => {
                depth -= 1;
                if depth == 0 {
                    return t[idx + 1..].trim_start();
                }
            }
            _ => {}
        }
    }
    t
}

/// Parse one joined field declaration — `pub(crate) below_count: Vec<u16>,` — into `(name, type)`.
///
/// Returns `None` for anything that is not a named field: an attribute, a `where` clause fragment,
/// a stray brace. The name must be lowercase-initial or `_`, which is what keeps a braced enum
/// variant (`Sphere { … }`) from being read as a field of its own enum.
fn parse_field(decl: &str) -> Option<(String, String)> {
    let mut t = decl.trim();
    loop {
        let before = t;
        for pre in ["pub(crate) ", "pub(super) ", "pub(in crate) ", "pub ", "mut "] {
            t = t.strip_prefix(pre).unwrap_or(t).trim_start();
        }
        if t == before {
            break;
        }
    }
    // `pub(crate)` with unusual spacing, e.g. `pub(crate)  name`.
    if let Some(rest) = t.strip_prefix("pub(") {
        let close = rest.find(')')?;
        t = rest[close + 1..].trim_start();
    }
    let name: String = t.chars().take_while(|c| is_ident_char(*c)).collect();
    if name.is_empty() || !name.starts_with(|c: char| c.is_ascii_lowercase() || c == '_') {
        return None;
    }
    let after = t[name.len()..].trim_start();
    let ty = after.strip_prefix(':')?.trim();
    // `::` is a path, not a field separator — `foo::bar` is not a declaration.
    if ty.starts_with(':') {
        return None;
    }
    let ty = ty.trim_end_matches(',').trim();
    if ty.is_empty() {
        return None;
    }
    Some((name, ty.to_string()))
}

/// Split a declaration list on the commas that are NOT inside `<…>`, `(…)` or `[…]`.
///
/// `a: Vec<u32>, b: [u32; 4]` splits in two; `m: Map<u32, u32>` does not split at all. Without the
/// nesting count a generic argument list would be read as a field boundary and both halves would
/// parse as junk.
fn split_top_level_commas(body: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (idx, c) in body.char_indices() {
        match c {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth -= 1,
            ',' if depth <= 0 => {
                out.push(&body[start..idx]);
                start = idx + c.len_utf8();
            }
            _ => {}
        }
    }
    if start < body.len() {
        out.push(&body[start..]);
    }
    out
}

/// Byte index of the first `=` or `;` in `ty` outside `<…>`, `(…)` and `[…]` — where a `static`
/// item's declared type ends and its initializer (or, for an `extern` static, the item) begins —
/// or `None` when the text ends first.
///
/// `[T; N]`'s `;` sits inside the brackets and `Box<dyn Iterator<Item = T>>`'s `=` inside the
/// angles, so neither ends the type early. The `>` of a `fn` pointer's `->` closes nothing.
fn type_end(ty: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut prev = ' ';
    for (idx, c) in ty.char_indices() {
        match c {
            '<' | '(' | '[' => depth += 1,
            '>' if prev == '-' => {}
            '>' | ')' | ']' => depth -= 1,
            '=' | ';' if depth <= 0 => return Some(idx),
            _ => {}
        }
        prev = c;
    }
    None
}

/// One `static` item that starts on a line.
struct StaticItem {
    /// The static's name.
    name: String,
    /// Its declared type, as far as the line carries it.
    ty: String,
    /// Whether the type ended on the line (an `=` or `;` was reached).
    closed: bool,
}

/// Every `static NAME: TYPE` item that starts in `code`, which has had its `//` comment stripped.
///
/// Keyed on the declaration shape — the token `static`, an optional `mut`, a name, one `:` —
/// anywhere on the line, so `pub(crate) static`, `static mut`, a static inside a `fn` body and the
/// one-line `thread_local! { static A: T = …; static B: U = …; }` are all read. A `static` right
/// after `'` is a lifetime (`&'static str`, `T: 'static`), not an item, and string bodies are
/// removed first so a literal cannot declare one. Only the last item on a line can be left open.
fn static_items(code: &str) -> Vec<StaticItem> {
    const KEYWORD: &str = "static";
    let code = strip_strings(code);
    let bytes = code.as_bytes();
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(offset) = code[from..].find(KEYWORD) {
        let start = from + offset;
        let end = start + KEYWORD.len();
        from = end;
        let before_ok = start == 0 || {
            let b = bytes[start - 1] as char;
            !is_ident_char(b) && b != '\''
        };
        let after_ok = code[end..].chars().next().is_none_or(|c| !is_ident_char(c));
        if !(before_ok && after_ok) {
            continue;
        }
        let mut rest = code[end..].trim_start();
        if let Some(after_mut) = rest.strip_prefix("mut")
            && after_mut.starts_with(char::is_whitespace)
        {
            rest = after_mut.trim_start();
        }
        let name: String = rest.chars().take_while(|c| is_ident_char(*c)).collect();
        if name.is_empty() || name.starts_with(|c: char| c.is_ascii_digit()) {
            continue;
        }
        let Some(ty) = rest[name.len()..].trim_start().strip_prefix(':') else {
            continue;
        };
        if ty.starts_with(':') {
            continue;
        }
        match type_end(ty) {
            Some(stop) => {
                out.push(StaticItem {
                    name,
                    ty: ty[..stop].trim().to_string(),
                    closed: true,
                });
                // `ty` is a suffix of `code`, so its offset is the length difference.
                from = code.len() - ty.len() + stop;
            }
            None => {
                out.push(StaticItem {
                    name,
                    ty: ty.trim().to_string(),
                    closed: false,
                });
                break;
            }
        }
    }
    out
}

/// Read a body written entirely on one line — `Edge { a: usize, b: usize },`, a live shape in
/// `narrowphase/box_box.rs`.
///
/// The multi-line path keys on a line that OPENS a brace, so a body that opens and closes on the
/// same line pushes nothing and its fields would never be counted. That would understate the field
/// denominator the anti-vacuity floor is calibrated against, and — worse — would be a one-line way
/// to declare `pub struct Foo { v: Vec<u32> }` past this gate.
fn scan_inline_body(rel: &str, lineno: usize, owner: &str, t: &str, census: &mut Census) {
    let (Some(open_at), Some(close_at)) = (t.find('{'), t.rfind('}')) else {
        return;
    };
    if close_at <= open_at {
        return;
    }
    for part in split_top_level_commas(&t[open_at + 1..close_at]) {
        let Some((name, ty)) = parse_field(part.trim()) else {
            continue;
        };
        census.fields += 1;
        if ty.contains(VEC_NEEDLE) {
            census.vec_fields.push(VecField {
                file: rel.to_string(),
                line: lineno,
                owner: owner.to_string(),
                field: name,
                ty,
            });
        }
    }
}

/// Scan one file, appending to `census`.
fn scan_file(rel: &str, text: &str, census: &mut Census) {
    let lines: Vec<&str> = text.lines().collect();
    let mut depth: i32 = 0;
    // Depth to return to before `#[cfg(test)]`-gated code stops being suppressed.
    let mut cfg_test_skip: Option<i32> = None;
    let mut pending_cfg_test = false;
    // Open field-bearing bodies: `(owner, the depth its fields sit at)`.
    let mut open: Vec<(String, i32)> = Vec::new();
    // Open enum bodies: `(name, the depth its variants sit at)`.
    let mut enums: Vec<(String, i32)> = Vec::new();
    // A `struct NAME` whose `{` has not arrived yet, because a `where` clause intervened.
    let mut pending_owner: Option<String> = None;
    // A field declaration being joined across lines, and the line its join started on.
    let mut pending_field = String::new();
    let mut pending_lines = 0usize;

    for (idx, raw) in lines.iter().enumerate() {
        let lineno = idx + 1;
        let code = strip_line_comment(raw);
        let t = code.trim();
        let before = depth;
        depth += brace_delta(&code);

        if let Some(floor) = cfg_test_skip {
            if depth <= floor {
                cfg_test_skip = None;
            }
            continue;
        }
        if pending_cfg_test {
            if depth > before {
                // The attribute introduced a block (`mod tests { … }`, `fn … { … }`): suppress it
                // whole.
                cfg_test_skip = Some(before);
                census.cfg_test_regions += 1;
                pending_cfg_test = false;
                continue;
            }
            if !t.starts_with("#[") && !t.is_empty() {
                // A single-line `#[cfg(test)]` item, or the attribute stack continuing.
                pending_cfg_test = false;
            }
        }
        if is_cfg_test_attr(t) {
            pending_cfg_test = true;
            continue;
        }

        // Close bodies the `}` on this line ended, flushing any field left mid-join (a last field
        // written without its trailing comma).
        while let Some((owner, field_depth)) = open.last().cloned() {
            if depth >= field_depth {
                break;
            }
            if !pending_field.is_empty() {
                if let Some((name, ty)) = parse_field(&pending_field) {
                    census.fields += 1;
                    if ty.contains(VEC_NEEDLE) {
                        census.vec_fields.push(VecField {
                            file: rel.to_string(),
                            line: lineno,
                            owner: owner.clone(),
                            field: name,
                            ty,
                        });
                    }
                }
                pending_field.clear();
                pending_lines = 0;
            }
            open.pop();
        }
        while enums.last().is_some_and(|(_, d)| depth < *d) {
            enums.pop();
        }

        // A `static` outlives every frame whatever scope declares it, so it is read at any depth —
        // module level, a `thread_local!` block, a `fn` body — not only inside an open body.
        let statics = static_items(t);
        if !statics.is_empty() {
            for item in statics {
                let mut ty = item.ty;
                if !item.closed {
                    let mut end = idx;
                    while end + 1 < lines.len() && end - idx < STATIC_JOIN_LOOKAHEAD {
                        end += 1;
                        let next = strip_strings(&strip_line_comment(lines[end]));
                        let joined = format!("{ty} {}", next.trim());
                        if let Some(stop) = type_end(&joined) {
                            ty = joined[..stop].trim().to_string();
                            break;
                        }
                        ty = joined;
                    }
                }
                census.statics += 1;
                if ty.contains(VEC_NEEDLE) {
                    census.vec_fields.push(VecField {
                        file: rel.to_string(),
                        line: lineno,
                        owner: item.name,
                        field: "<static>".to_string(),
                        ty,
                    });
                }
            }
            continue;
        }

        // A `struct NAME` whose brace arrives on a later line.
        if let Some(name) = pending_owner.clone()
            && depth > before
            && t.contains('{')
        {
            open.push((name, before + 1));
            census.containers += 1;
            pending_owner = None;
            pending_field.clear();
            continue;
        }

        if let Some((name, tail)) = declared(t, "struct") {
            let tail = skip_generics(tail);
            if tail.starts_with('(') {
                // Tuple struct. Join to its `;` and treat the whole positional list as one field:
                // the fields have no names to report, and one `Vec<` anywhere in it is the finding.
                census.containers += 1;
                let mut decl = t.to_string();
                let mut end = idx;
                while !decl.contains(';') && end + 1 < lines.len() && end - idx < TUPLE_JOIN_LOOKAHEAD
                {
                    end += 1;
                    decl.push(' ');
                    decl.push_str(strip_line_comment(lines[end]).trim());
                }
                census.fields += 1;
                if decl.contains(VEC_NEEDLE) {
                    census.vec_fields.push(VecField {
                        file: rel.to_string(),
                        line: lineno,
                        owner: name,
                        field: "<positional>".to_string(),
                        ty: decl.trim().to_string(),
                    });
                }
                continue;
            }
            if tail.starts_with(';') {
                // Unit struct: a container with no fields, counted so the denominator is honest.
                census.containers += 1;
                continue;
            }
            if tail.starts_with('{') {
                census.containers += 1;
                if depth > before {
                    open.push((name, before + 1));
                    pending_field.clear();
                } else {
                    // Opened and closed on this line.
                    scan_inline_body(rel, lineno, &name, t, census);
                }
                continue;
            }
            // `struct Foo<T>` with a `where` clause below it.
            pending_owner = Some(name);
            continue;
        }

        if let Some((name, tail)) = declared(t, "enum")
            && depth > before
            && skip_generics(tail).contains('{')
        {
            enums.push((name, before + 1));
            continue;
        }

        // A braced enum variant is a field-bearing body too.
        if let Some((enum_name, variant_depth)) = enums.last().cloned()
            && before == variant_depth
            && t.contains('{')
            && t.starts_with(|c: char| c.is_ascii_uppercase())
        {
            let variant: String = t.chars().take_while(|c| is_ident_char(*c)).collect();
            let owner = format!("{enum_name}::{variant}");
            census.containers += 1;
            if depth > before {
                open.push((owner, before + 1));
                pending_field.clear();
            } else {
                // `Edge { a: usize, b: usize },` — the live one-line variant shape.
                scan_inline_body(rel, lineno, &owner, t, census);
            }
            continue;
        }

        // Field declarations sit at exactly the open body's field depth and open no braces of
        // their own — no Rust type contains one.
        let Some((owner, field_depth)) = open.last().cloned() else {
            continue;
        };
        if before != field_depth || depth != before {
            continue;
        }
        if t.is_empty() || t.starts_with("#[") {
            continue;
        }
        if pending_field.is_empty() {
            pending_field = t.to_string();
        } else {
            pending_field.push(' ');
            pending_field.push_str(t);
        }
        pending_lines += 1;
        if !pending_field.ends_with(',') && pending_lines < FIELD_JOIN_LOOKAHEAD {
            continue;
        }
        if let Some((name, ty)) = parse_field(&pending_field) {
            census.fields += 1;
            if ty.contains(VEC_NEEDLE) {
                census.vec_fields.push(VecField {
                    file: rel.to_string(),
                    line: lineno,
                    owner: owner.clone(),
                    field: name,
                    ty,
                });
            }
        }
        pending_field.clear();
        pending_lines = 0;
    }
}

/// One out-of-line `mod NAME;` item at the top level of a file, with what its attribute stack
/// says.
struct ModDecl {
    /// The declared module name.
    name: String,
    /// The value of a `#[path = "…"]` attribute in its stack, if any.
    path: Option<String>,
    /// Whether its stack carries a `#[cfg(…)]` that [`is_cfg_test_attr`] reads as a test cfg.
    cfg_test: bool,
    /// 1-indexed line of the `mod` item.
    line: usize,
}

/// The name declared by `<visibility> mod NAME;`, or `None` for anything else — including an
/// inline `mod NAME {`, whose body the region skipper already handles.
fn out_of_line_mod(code: &str) -> Option<String> {
    let mut t = code.trim();
    if let Some(rest) = t.strip_prefix("pub(") {
        t = rest.split_once(')')?.1.trim_start();
    } else if let Some(rest) = t.strip_prefix("pub ") {
        t = rest.trim_start();
    }
    let name = t.strip_prefix("mod ")?.trim().strip_suffix(';')?.trim_end();
    (!name.is_empty() && name.chars().all(is_ident_char)).then(|| name.to_string())
}

/// The value of a `#[path = "…"]` attribute line.
fn path_attr(trimmed: &str) -> Option<String> {
    let rest = trimmed
        .strip_prefix("#[path")?
        .trim_start()
        .strip_prefix('=')?
        .trim_start()
        .strip_prefix('"')?;
    Some(rest[..rest.find('"')?].to_string())
}

/// Every `mod NAME;` at brace depth 0 of one file, with its attribute stack read upward.
///
/// The stack ends at the first line that is neither an attribute nor a comment — so a blank line
/// inside it, or an item above it, ends it. Ending early can only hide a test cfg, which leaves
/// the declared file SCANNED: the conservative direction.
fn mod_decls(text: &str) -> Vec<ModDecl> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut depth = 0i32;
    for (idx, raw) in lines.iter().enumerate() {
        let code = strip_line_comment(raw);
        let before = depth;
        depth += brace_delta(&code);
        if before != 0 {
            continue;
        }
        let Some(name) = out_of_line_mod(&code) else {
            continue;
        };
        let mut path = None;
        let mut cfg_test = false;
        for above in lines[..idx].iter().rev() {
            let t = above.trim();
            if t.starts_with("//") {
                continue;
            }
            if !t.starts_with("#[") {
                break;
            }
            let attr = strip_line_comment(t);
            let attr = attr.trim();
            cfg_test |= is_cfg_test_attr(attr);
            if let Some(p) = path_attr(attr) {
                path = Some(p);
            }
        }
        out.push(ModDecl {
            name,
            path,
            cfg_test,
            line: idx + 1,
        });
    }
    out
}

/// The files `decl`, declared in `declaring`, can load, in Rust's lookup order.
///
/// A `#[path]` is relative to the declaring file's directory. Otherwise the module sits beside a
/// `mod.rs` / `lib.rs` / `main.rs`, or under `<stem>/` beside any other file, as `NAME.rs` or
/// `NAME/mod.rs`.
fn mod_file_candidates(declaring: &str, decl: &ModDecl) -> Vec<String> {
    let (dir, file) = declaring.rsplit_once('/').unwrap_or(("", declaring));
    let join = |p: &str| {
        if dir.is_empty() {
            p.to_string()
        } else {
            format!("{dir}/{p}")
        }
    };
    if let Some(p) = &decl.path {
        return vec![join(p)];
    }
    let name = &decl.name;
    if matches!(file, "mod.rs" | "lib.rs" | "main.rs") {
        vec![join(&format!("{name}.rs")), join(&format!("{name}/mod.rs"))]
    } else {
        let stem = file.strip_suffix(".rs").unwrap_or(file);
        vec![
            join(&format!("{stem}/{name}.rs")),
            join(&format!("{stem}/{name}/mod.rs")),
        ]
    }
}

/// Files the crate loads only under `cfg(test)`, each mapped to the `file:line` of the `mod` item
/// that gates it.
///
/// A file pulled in by `#[cfg(test)] mod tests;` carries no cfg of its own, so [`scan_file`]'s
/// region skipper cannot see it: the signal sits on the DECLARING item, and this reads it there.
/// Only a top-level `mod NAME;` with a test cfg in its stack counts, only when it resolves to a
/// scanned file, and only when no top-level live `mod` item resolves to that file as well — a
/// `#[cfg(test)] #[path]` item that re-loads a live module must not take the live module with it.
/// Every miss leaves a file scanned — a false red, never a false green.
fn cfg_test_module_files(sources: &[(String, String)]) -> BTreeMap<String, String> {
    let scanned: BTreeSet<&str> = sources.iter().map(|(rel, _)| rel.as_str()).collect();
    let mut gated = BTreeMap::new();
    let mut live = BTreeSet::new();
    for (rel, text) in sources {
        for decl in mod_decls(text) {
            let Some(file) = mod_file_candidates(rel, &decl)
                .into_iter()
                .find(|c| scanned.contains(c.as_str()))
            else {
                continue;
            };
            if decl.cfg_test {
                gated.insert(file, format!("{rel}:{}", decl.line));
            } else {
                live.insert(file);
            }
        }
    }
    gated.retain(|file, _| !live.contains(file));
    gated
}

/// Every `.rs` file under `dir`, repo-relative and `/`-separated, in a stable order.
fn collect_rs(dir: &Path, root: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect_rs(&path, root, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(
                path.strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
}

/// Scan [`SCANNED_ROOT`] once.
fn census() -> Census {
    let root = repo_root();
    let scanned = root.join(SCANNED_ROOT);
    assert!(
        scanned.is_dir(),
        "SCANNED_ROOT does not exist: {}. The gate scans nothing, which would read as a clean \
         tree — if boyko_physics moved, move this constant with it.",
        scanned.display()
    );

    let mut files = Vec::new();
    collect_rs(&scanned, &root, &mut files);
    let sources: Vec<(String, String)> = files
        .into_iter()
        .map(|rel| {
            let Ok(text) = std::fs::read_to_string(root.join(&rel)) else {
                panic!("unreadable source file under {SCANNED_ROOT}: {rel}");
            };
            (rel, text)
        })
        .collect();
    let test_modules = cfg_test_module_files(&sources);

    let mut out = Census {
        files: 0,
        skipped_test_files: Vec::new(),
        cfg_test_regions: 0,
        containers: 0,
        fields: 0,
        statics: 0,
        vec_fields: Vec::new(),
    };
    for (rel, text) in &sources {
        // A test module in a file of its own (`#[cfg(test)] mod tests;`, with or without a
        // `#[path]`) carries no cfg attribute itself; the declaring item does.
        if let Some(site) = test_modules.get(rel) {
            out.skipped_test_files.push(format!("{rel} <- {site}"));
            continue;
        }
        out.files += 1;
        scan_file(rel, text, &mut out);
    }
    out
}

/// Print every count the gate enforced, so no figure in this file's prose has to be remembered.
fn report(c: &Census) {
    println!(
        "[physics Vec side-store census] root={SCANNED_ROOT}  {} file(s) scanned, {} \
         `#[cfg(test)]` module file(s) skipped ({:?}), {} `#[cfg(test)]` region(s) suppressed, {} \
         field-bearing container(s), {} field declaration(s) and {} `static` item(s) parsed, {} \
         of them `Vec`-typed (pin {PINNED_VEC_FIELDS}), {} exception row(s)",
        c.files,
        c.skipped_test_files.len(),
        c.skipped_test_files,
        c.cfg_test_regions,
        c.containers,
        c.fields,
        c.statics,
        c.vec_fields.len(),
        KNOWN_VEC_FIELD_SITES.len(),
    );
    for a in KNOWN_VEC_FIELD_SITES {
        let found = c
            .vec_fields
            .iter()
            .filter(|v| v.file == a.file && v.owner == a.owner)
            .count();
        println!(
            "[physics Vec side-store census]   {}::{} — {found} found, {} allowed",
            a.file, a.owner, a.fields
        );
    }
}

/// What the predicate cannot decide, restated wherever it fails — because a reader meeting this
/// gate for the first time is meeting it in a failure message, not in the module doc.
const NARROWING: &str = "\
WHAT THIS GATE DOES AND DOES NOT DECIDE:\n\
\x20 * It forbids the SHAPE (`Vec<` in the declared type of a struct field or a `static` item), \
because the actual rule — \
durable subsystem data must live in ECS storage — is NOT decidable from a declaration. A frame \
scratch buffer and a per-entity mirror are spelled identically.\n\
\x20 * The judgement therefore lives in KNOWN_VEC_FIELD_SITES, one row per site with the rung that \
removes it — the same discipline as `// SAFETY:` and the `#[allow(clippy::disallowed_types)]` \
rationales.\n\
\x20 * A `static` item IS a finding when its type mentions `Vec<` — at module level, in a \
`thread_local!` block or in a `fn` body — because a static outlives every frame. It is reported as \
`NAME::<static>`; a row excusing one names the static as the owner, with a count of 1.\n\
\x20 * A `Vec` in a function signature, a local or a return type is NOT a finding here and never \
was: CLAUDE.md names function-local scratch as legitimate. That includes an ECS `Local<Vec<_>>` \
system parameter, which is persistent (zero of them in boyko_physics on the B4 trunk).\n\
\x20 * The match is textual. A field written through an alias (`type Rows = Vec<u32>`), a \
use-rename (`use std::vec::Vec as Column`) or a type-position turbofish (`Vec::<u32>`) is \
invisible to it, and so is a `VecDeque` (zero of each in boyko_physics on the B4 trunk).";

/// The ban: no `Vec`-typed field in `boyko_physics` outside the enumerated sites.
#[test]
fn no_unlisted_vec_side_store_in_boyko_physics() {
    let c = census();
    report(&c);

    // ── Anti-vacuity guard 1: the walk and the field parser both ran.
    assert!(
        c.files > 0 && c.containers >= MIN_CONTAINERS,
        "the scan of {SCANNED_ROOT} entered only {} field-bearing container(s) across {} file(s) \
         (floor {MIN_CONTAINERS}). The walker or the struct detector is broken, not the tree — a \
         census that scans nothing reports a triumphant zero.",
        c.containers,
        c.files
    );
    assert!(
        c.fields >= MIN_FIELDS,
        "the scan parsed only {} field declaration(s) (floor {MIN_FIELDS}). The field parser has \
         stopped parsing; every clean run after this point would be clean from emptiness.",
        c.fields
    );
    assert!(
        c.statics >= MIN_STATICS,
        "the scan read only {} `static` item(s) (floor {MIN_STATICS}). The static-item reader has \
         stopped reading. No static in {SCANNED_ROOT} is `Vec`-typed today, so the `Vec` count \
         cannot notice this: a `Vec` in a `static` or a `thread_local!` would pass unseen from \
         here on.",
        c.statics
    );

    // ── Anti-vacuity guard 2: the type matcher still matches something.
    assert!(
        !c.vec_fields.is_empty(),
        "the scan found ZERO `Vec`-typed fields under {SCANNED_ROOT}. That is not a clean tree: \
         {} exception row(s) name sites that are supposed to be found. Either every side store \
         was migrated in one commit — in which case delete the rows, deliberately — or the type \
         matcher is broken.",
        KNOWN_VEC_FIELD_SITES.len()
    );

    // ── The clause itself.
    let allowed: BTreeSet<(&str, &str)> = KNOWN_VEC_FIELD_SITES
        .iter()
        .map(|a| (a.file, a.owner))
        .collect();
    let unlisted: Vec<String> = c
        .vec_fields
        .iter()
        .filter(|v| !allowed.contains(&(v.file.as_str(), v.owner.as_str())))
        .map(|v| format!("{}:{}  `{}::{}: {}`", v.file, v.line, v.owner, v.field, v.ty))
        .collect();

    assert!(
        unlisted.is_empty(),
        "a NEW `std::Vec` side store has appeared in boyko_physics:\n  {}\n\n\
         CLAUDE.md principle 0: durable per-entity / per-element / bulk subsystem data lives in \
         the ECS's own storage — `ComponentPool` columns, `Resource`-owned columns, or dense \
         components — \"never `std::Vec` / `HashMap` as a side store\". The precedent is in the \
         same paragraph: a `std::Vec` physics mirror caused the O11-SP4 colored-solve data race.\n\n\
         WHAT TO DO — pick one, in this order:\n\
         \x20 1. Put the data in the kernel. A `ScratchColumn` for per-step scratch (235 lines \
         across this crate already on the B4 trunk), a `Resource`-owned column or a dense \
         component for durable per-element state. That is the outcome this gate exists to \
         produce.\n\
         \x20 2. If it is genuinely function-local scratch, it does not belong in a struct field \
         or a `static` at all — make it a local. If it is test code in a file of its own, declare \
         that file with `#[cfg(test)] mod …;` and the scan skips it.\n\
         \x20 3. Only if neither holds, add a row to KNOWN_VEC_FIELD_SITES in \
         tests/physics_vec_side_store_census.rs naming the struct, its exact field count, and THE \
         RUNG THAT REMOVES IT, and raise PINNED_VEC_FIELDS by the same count — the unified plan \
         reads that pin, so a raise is a recorded finding, not a tidy-up. A row without a named \
         rung is a parking space, and the one row there today is a named rung (S0; L10 C0 \
         retired U6's `IslandSleep` row).\n\n{NARROWING}",
        unlisted.join("\n  ")
    );

    // ── The pin, in the physics design's own words (R0): red if the count is not the pin (30).
    assert!(
        c.vec_fields.len() == PINNED_VEC_FIELDS,
        "the scan found {} `Vec`-typed field(s) under {SCANNED_ROOT}; the pin is \
         {PINNED_VEC_FIELDS}. A `static` item whose type mentions `Vec<` counts as one field. \
         Every field found is on KNOWN_VEC_FIELD_SITES (the clause above \
         passed), so the difference is inside a listed struct, and \
         `every_known_vec_side_store_is_present_at_its_declared_count` names it. The unified plan \
         reads this pin (L10 C0: 34 → 30, ahead of U6; S0: 30 → 0): lower it and the row \
         together, in the commit \
         that migrates the store, never to meet green.\n\n{NARROWING}",
        c.vec_fields.len()
    );
}

/// The other half: every enumerated site must still be there, at exactly the count it claims.
///
/// Without this the list is a snapshot — it would keep reading as coverage after its subject grew
/// a field, shed one, or vanished entirely. Anti-vacuity guard 3 lives here, because "the code the
/// gate points at was deleted" and "the gate stopped finding it" are indistinguishable from the
/// outside and both must be loud.
#[test]
fn every_known_vec_side_store_is_present_at_its_declared_count() {
    let c = census();
    report(&c);

    let mut report_text = String::new();
    for a in KNOWN_VEC_FIELD_SITES {
        let found: Vec<&VecField> = c
            .vec_fields
            .iter()
            .filter(|v| v.file == a.file && v.owner == a.owner)
            .collect();

        // Guard 3, stated separately from the count check: a site that yields NOTHING is either
        // deleted or invisible to the scanner, and neither may pass quietly.
        if found.is_empty() {
            report_text.push_str(&format!(
                "\n{}::{} — the gate found NO `Vec` field there at all (expected {}).\n\
                 Either the struct was migrated or renamed — in which case DELETE the row, which \
                 is the good outcome and must still be a deliberate edit — or the scanner has \
                 stopped seeing it, in which case every green run from here on is green from \
                 emptiness. Rung on file: {}\n",
                a.file, a.owner, a.fields, a.rung
            ));
            continue;
        }
        if found.len() != a.fields {
            let direction = if found.len() > a.fields {
                "has GAINED"
            } else {
                "has SHED"
            };
            let delta = found.len().abs_diff(a.fields);
            report_text.push_str(&format!(
                "\n{}::{} {direction} {delta} `Vec` field(s): {} found, {} allowed.\n\
                 The exception list is EXACT, not a prefix — that is the half that makes it more \
                 than a snapshot. A gain is a NEW side store hiding inside a struct that was \
                 already excused; a loss means the row over-states its subject and reads as \
                 coverage it no longer has. Fix the code, or re-derive the row's count from this \
                 run — never the other way round. Rung on file: {}\n  {}\n",
                a.file,
                a.owner,
                found.len(),
                a.fields,
                a.rung,
                found
                    .iter()
                    .map(|v| format!("{}:{}  `{}: {}`", v.file, v.line, v.field, v.ty))
                    .collect::<Vec<_>>()
                    .join("\n  ")
            ));
        }
    }

    assert!(
        report_text.is_empty(),
        "KNOWN_VEC_FIELD_SITES no longer describes the tree.{report_text}\n{NARROWING}"
    );
}

/// The scanner's own positive control: every shape it will meet, classified by hand.
///
/// The floors in the ban test catch a scanner that found *nothing*. They cannot catch one that
/// reads `pub prev_x: Vec<f32>,` as a non-field, or reads a doc comment quoting `Vec<f32>` as a
/// declaration — failures invisible from the outside that look exactly like a clean crate. This is
/// why the sibling `ignore_reasons_census.rs` carries a hand-classified table too.
#[test]
fn the_field_parser_reads_every_shape_it_will_meet() {
    let cases: &[(&str, Option<(&str, &str)>)] = &[
        // The live shapes.
        ("pub pos_x: Vec<f32>,", Some(("pos_x", "Vec<f32>"))),
        ("asleep: Vec<bool>,", Some(("asleep", "Vec<bool>"))),
        ("pub(crate) below_count: Vec<u16>,", Some(("below_count", "Vec<u16>"))),
        // No trailing comma — the last field of a body, flushed when the body closes.
        ("pub energy: Vec<f32>", Some(("energy", "Vec<f32>"))),
        // Nested and qualified spellings that must still be seen as `Vec`.
        ("pub rows: Option<Vec<u32>>,", Some(("rows", "Option<Vec<u32>>"))),
        ("pub rows: std::vec::Vec<u32>,", Some(("rows", "std::vec::Vec<u32>"))),
        ("pub lanes: [Vec<u32>; 4],", Some(("lanes", "[Vec<u32>; 4]"))),
        // Non-`Vec` fields: parsed (they carry the denominator) but not findings.
        ("pub count: usize,", Some(("count", "usize"))),
        ("pub cols: ScratchColumn<f32>,", Some(("cols", "ScratchColumn<f32>"))),
        ("pub queue: VecDeque<u32>,", Some(("queue", "VecDeque<u32>"))),
        // Not fields at all.
        ("#[derive(Component, Clone, Debug, Default)]", None),
        ("Sphere {", None),
        ("}", None),
        ("where", None),
        ("T: Bar,", None),
    ];
    for (line, want) in cases {
        let got = parse_field(line);
        let got_ref = got.as_ref().map(|(n, t)| (n.as_str(), t.as_str()));
        assert_eq!(got_ref, *want, "parse_field({line:?}) read the wrong thing");
    }

    // A `Vec` mention that is NOT a field type must never become a finding — this is the whole
    // reason the scanner reads declarations rather than grepping for `Vec<`.
    for line in [
        "fn build(&mut self, out: &mut Vec<u32>) {",
        "let mut scratch: Vec<u32> = Vec::new();",
        "/// across `x`/`y`/`z` `Vec<f32>` so each substep streams a tight, vectorizable",
    ] {
        let code = strip_line_comment(line);
        assert!(
            parse_field(code.trim()).is_none_or(|(_, ty)| !ty.contains(VEC_NEEDLE)),
            "a non-field `Vec` mention was read as a field declaration: {line:?}"
        );
    }

    // `static` items: every declaration shape read with its type cut at the right `=` / `;`, and
    // a lifetime, a string literal or a longer identifier never read as one.
    type Want<'a> = &'a [(&'a str, &'a str, bool)];
    let static_cases: &[(&str, Want)] = &[
        (
            "pub static ROWS: std::sync::OnceLock<Vec<u32>> = std::sync::OnceLock::new();",
            &[("ROWS", "std::sync::OnceLock<Vec<u32>>", true)],
        ),
        (
            "pub(crate) static mut COUNT: u64 = 0;",
            &[("COUNT", "u64", true)],
        ),
        (
            "pub static SPAN_ZONES: [&ZoneHandle; SPAN_ZONE_COUNT] = [",
            &[("SPAN_ZONES", "[&ZoneHandle; SPAN_ZONE_COUNT]", true)],
        ),
        (
            "thread_local! { static A: Cell<Vec<u8>> = const { Cell::new(Vec::new()) }; \
             static B: Cell<u64> = const { Cell::new(0) }; }",
            &[("A", "Cell<Vec<u8>>", true), ("B", "Cell<u64>", true)],
        ),
        (
            "static ITER: Box<dyn Iterator<Item = Vec<u32>> + Sync> = make();",
            &[("ITER", "Box<dyn Iterator<Item = Vec<u32>> + Sync>", true)],
        ),
        (
            "static F: fn(u32) -> Vec<u32> = f;",
            &[("F", "fn(u32) -> Vec<u32>", true)],
        ),
        (
            "unsafe extern \"C\" { safe static EXT: u32; }",
            &[("EXT", "u32", true)],
        ),
        (
            "static WRAPPED: std::cell::RefCell<",
            &[("WRAPPED", "std::cell::RefCell<", false)],
        ),
        ("pub fn f(x: &'static str) -> &'static [u32] {", &[]),
        ("where T: 'static,", &[]),
        ("let s = \"static FAKE: Vec<u32> = x;\";", &[]),
        ("pub is_static: Vec<bool>,", &[]),
        ("static::helper();", &[]),
    ];
    for (line, want) in static_cases {
        let got: Vec<(String, String, bool)> = static_items(line)
            .into_iter()
            .map(|s| (s.name, s.ty, s.closed))
            .collect();
        let got_ref: Vec<(&str, &str, bool)> = got
            .iter()
            .map(|(n, t, closed)| (n.as_str(), t.as_str(), *closed))
            .collect();
        assert_eq!(
            got_ref.as_slice(),
            *want,
            "static_items({line:?}) read the wrong thing"
        );
    }

    // Region suppression: every cfg spelling that gates test code in this crate.
    for line in [
        "#[cfg(test)]",
        "    #[cfg(test)]",
        "#[cfg(all(test, target_arch = \"x86_64\", target_feature = \"avx2\"))]",
        "#[cfg(all(test, not(miri)))]",
        "#[cfg(any(test, feature = \"probe\"))]",
    ] {
        assert!(is_cfg_test_attr(line.trim()), "not read as a test cfg: {line:?}");
    }
    for line in [
        "#[cfg(debug_assertions)]",
        "#[cfg(target_feature = \"avx2\")]",
        "#[cfg(feature = \"test_only\")]",
        "#[derive(Component)]",
    ] {
        assert!(
            !is_cfg_test_attr(line.trim()),
            "wrongly read as a test cfg — code would be suppressed and the gate would go blind \
             over it: {line:?}"
        );
    }
}

/// The scanner's end-to-end control, on a synthetic file carrying every container shape.
///
/// `the_field_parser_reads_every_shape_it_will_meet` proves the line parser; this proves the state
/// machine around it — the brace tracking, the `#[cfg(test)]` region skip, the three
/// field-bearing container shapes and the `static` item reader. A tuple struct, a braced enum
/// variant and a `static` carry no `Vec` in `boyko_physics` today, so the live corpus cannot
/// exercise those paths at all: without this control they would be code that has never once run
/// against a positive case.
#[test]
fn the_scanner_sees_every_container_shape_and_skips_test_regions() {
    let source = concat!(
        "pub struct Braced {\n",
        "    /// A doc comment mentioning `Vec<f32>` must not be read as a field.\n",
        "    pub kept: Vec<f32>,\n",
        "    pub plain: usize,\n",
        "}\n",
        "pub struct Tuple(pub Vec<u32>);\n",
        "pub struct Unit;\n",
        "pub enum Shape {\n",
        "    Sphere { radius: f32 },\n",
        "    Mesh { verts: Vec<u32> },\n",
        "    Edge(u32),\n",
        "}\n",
        "pub struct Generic<T>\n",
        "where\n",
        "    T: Copy,\n",
        "{\n",
        "    pub late: Vec<T>,\n",
        "}\n",
        "/// A doc comment naming `static DOC: Vec<u8>` declares nothing.\n",
        "pub static ROWS: std::sync::OnceLock<Vec<u32>> = std::sync::OnceLock::new();\n",
        "static mut COUNT: u64 = 0;\n",
        "thread_local! { static SCRATCH: std::cell::Cell<Vec<u32>> = const { \
         std::cell::Cell::new(Vec::new()) }; }\n",
        "thread_local! {\n",
        "    static WRAPPED: std::cell::RefCell<\n",
        "        Vec<u8>,\n",
        "    > = const { std::cell::RefCell::new(Vec::new()) };\n",
        "}\n",
        "pub fn lifetimes(x: &'static str) -> &'static str {\n",
        "    static LOCAL: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();\n",
        "    x\n",
        "}\n",
        "#[cfg(test)]\n",
        "mod tests {\n",
        "    pub struct Suppressed {\n",
        "        pub invisible: Vec<u8>,\n",
        "    }\n",
        "    static HIDDEN_INVISIBLE: Vec<u8> = Vec::new();\n",
        "}\n",
        "#[cfg(all(test, target_arch = \"x86_64\"))]\n",
        "fn helper() {\n",
        "    struct AlsoSuppressed {\n",
        "        also_invisible: Vec<u8>,\n",
        "    }\n",
        "}\n",
    );
    let mut c = Census {
        files: 0,
        skipped_test_files: Vec::new(),
        cfg_test_regions: 0,
        containers: 0,
        fields: 0,
        statics: 0,
        vec_fields: Vec::new(),
    };
    scan_file("synthetic.rs", source, &mut c);

    let found: Vec<String> = c
        .vec_fields
        .iter()
        .map(|v| format!("{}::{}", v.owner, v.field))
        .collect();
    assert_eq!(
        found,
        vec![
            "Braced::kept".to_string(),
            "Tuple::<positional>".to_string(),
            "Shape::Mesh::verts".to_string(),
            "Generic::late".to_string(),
            "ROWS::<static>".to_string(),
            "SCRATCH::<static>".to_string(),
            "WRAPPED::<static>".to_string(),
            "LOCAL::<static>".to_string(),
        ],
        "the scanner did not see every container shape (or saw a suppressed one). Missing \
         `Tuple::<positional>` means `struct Foo(Vec<u32>);` is a one-line way around this gate; \
         missing `Shape::Mesh::verts` means a braced enum variant is; missing a `<static>` means \
         a `static` / `thread_local!` item is (`WRAPPED`: a type wrapped across lines; `LOCAL`: a \
         static inside a `fn` body); seeing anything named `DOC` or `invisible` means a comment \
         was read as code or `#[cfg(test)]` suppression is broken in the direction that reds \
         honest trees."
    );
    assert_eq!(
        c.statics, 5,
        "`static` items read: ROWS, COUNT, SCRATCH, WRAPPED, LOCAL. The `&'static` lifetimes and \
         the doc comment are not items, and HIDDEN_INVISIBLE sits in a `#[cfg(test)]` region"
    );
    assert_eq!(
        c.cfg_test_regions, 2,
        "both `#[cfg(test)]` spellings must suppress a region — `#[cfg(test)] mod tests` and \
         `#[cfg(all(test, …))] fn helper`"
    );
    // Braced + Tuple + Unit + Shape::Sphere + Shape::Mesh + Generic. The enum itself bears no
    // fields and is not counted; `Edge(u32)` is a tuple variant, which this scanner does not read
    // (a narrowing: it costs nothing today, and the module doc says so).
    assert_eq!(
        c.containers, 6,
        "container count drifted — the denominator the MIN_CONTAINERS floor is calibrated against"
    );
}

/// The test-module file skip, on a synthetic crate: keyed on the declaring item's cfg, never on
/// the file's name.
///
/// The live tree exercises only the positive half (three gated files). This fixes the other
/// half too: a live module NAMED `tests.rs` stays scanned, a `feature = "test_only"` cfg does not
/// gate, a live module that a test `#[path]` item also loads stays scanned, and a `mod` nested in
/// an inline block is not resolved — the stated narrowing, which leaves its file scanned rather
/// than skipped.
#[test]
fn a_test_module_file_is_skipped_by_its_declaring_cfg_not_by_its_name() {
    let src = |rel: &str, text: &str| (rel.to_string(), text.to_string());
    let sources = vec![
        src(
            "c/src/lib.rs",
            concat!(
                "pub mod live;\n",
                "pub mod grid;\n",
                "pub mod tree;\n",
                "#[cfg(test)]\n",
                "mod t;\n",
                "#[cfg(all(test, not(miri)))]\n",
                "// a comment inside the attribute stack does not end it\n",
                "mod probe;\n",
                "#[cfg(feature = \"test_only\")]\n",
                "mod f;\n",
                "#[cfg(test)]\n",
                "mod inline {\n",
                "    mod nested;\n",
                "}\n",
                "pub mod util;\n",
            ),
        ),
        src(
            "c/src/other.rs",
            "#[cfg(test)]\n#[path = \"util.rs\"]\nmod util_under_test;\n",
        ),
        src("c/src/util.rs", ""),
        src(
            "c/src/live.rs",
            concat!(
                "#[cfg(test)]\n",
                "#[path = \"live_tests.rs\"]\n",
                "mod tests;\n",
                "pub mod inner;\n",
            ),
        ),
        src("c/src/grid/mod.rs", "#[cfg(test)]\nmod tests;\n"),
        src("c/src/tree/mod.rs", "pub(crate) mod tests;\n"),
        src("c/src/t.rs", ""),
        src("c/src/probe.rs", ""),
        src("c/src/f.rs", ""),
        src("c/src/live_tests.rs", ""),
        src("c/src/live/inner.rs", ""),
        src("c/src/grid/tests.rs", ""),
        src("c/src/tree/tests.rs", ""),
        src("c/src/inline/nested.rs", ""),
    ];
    let gated = cfg_test_module_files(&sources);
    let got: Vec<(&str, &str)> = gated
        .iter()
        .map(|(file, site)| (file.as_str(), site.as_str()))
        .collect();
    assert_eq!(
        got,
        vec![
            ("c/src/grid/tests.rs", "c/src/grid/mod.rs:2"),
            ("c/src/live_tests.rs", "c/src/live.rs:3"),
            ("c/src/probe.rs", "c/src/lib.rs:8"),
            ("c/src/t.rs", "c/src/lib.rs:5"),
        ],
        "the test-module skip resolved the wrong set. Missing `grid/tests.rs` or `live_tests.rs` \
         means one of the two live layouts is scanned as live code (a false red on the trunk); \
         seeing `tree/tests.rs` means a live module is skipped for its NAME, seeing `f.rs` \
         means a non-test cfg gates, and seeing `util.rs` means a live module is skipped because \
         a test `#[path]` item also loads it — all three false greens."
    );
}
