//! Census: a Gaia / Aether ballot that is RULED may not be claimed OPEN in the registers.
//!
//! # Why this file exists
//!
//! Ballot **GB-5** was ruled by the owner on 2026-08-30 and recorded on 2026-08-31 by
//! `b6c41237` — a commit that touched `docs/gaia/DECISIONS.md` **and nothing else**. Every
//! register kept saying the ballot was open: on `feat/threadpool-ke16` its
//! `docs/OPEN-QUESTIONS.md:250` (`⚠ **STAYS OPEN.**`), `:282` (`**Still open and genuinely
//! awaiting an answer: … GB-5 …**`), `:195-199` (a `### Owner ballots still open` heading at
//! `:195` over a paragraph naming `**GB-5**` at `:198`) and its
//! `docs/gaia/CAMPAIGN.md:124` (`⚠ **OPEN — the owner asked for the trade-offs,
//! not a ruling, and a ruling here would be a defect.**`). The consequence was not cosmetic:
//! G1's field-table freeze read as BLOCKED in every index while the log said UNBLOCKED.
//! [`docs/OPEN-QUESTIONS.md`](../docs/OPEN-QUESTIONS.md) §*2026-09-03*, the GB-5 section,
//! names the cure — *"a census asserting that no ballot id appearing under a RULED heading may
//! appear as OPEN"* — and records it as owed. This file is that census.
//!
//! # The predicate, and its narrowing
//!
//! **RULED** (the source set) is built from two shapes only:
//!
//! 1. a heading in `docs/gaia/DECISIONS.md` carrying an uppercase whole-word
//!    `RULED` / `RESOLVED` / `DISPOSED` / `RATIFIED` — every ballot id in the heading is ruled;
//! 2. a row of a table in `docs/OPEN-QUESTIONS.md` whose header has a `who` cell, whose first
//!    cell (stripped of `*` and spaces) is exactly a ballot id, and whose `who` cell — stripped,
//!    lower-cased — **starts with** `owner`, `delegated` or `standing rule` and contains none of
//!    `partly`, `asked`, `analys`. The ruling is read from the `who` column and NOT from a token
//!    in the ruling cell, because the ruling cells carry no uniform ruling word: F1 reads
//!    `**Macro-time GK-4, NOW**`, GB-5 reads `**PERMIT AS SEED**`, F4 reads `**(a), as
//!    SUPPRESS-THEN-FIXUP**` — the `who` column is the one signal every ruled row shares.
//!    `partly` excludes F9 (its residual is the owner's, and the registers say so); `asked` /
//!    `analys` exclude the ke16 `**owner asked for ANALYSIS**` shape, so a row cannot be its own
//!    contradiction by construction. A `who` spelling outside this closed vocabulary counts as
//!    NOT ruled — the safe direction: it can miss a red, never fabricate one — and the
//!    non-emptiness guard below catches wholesale drift.
//!
//! **OPEN** (the claim set) is read from `docs/OPEN-QUESTIONS.md` and `docs/gaia/CAMPAIGN.md`,
//! and **only from bold spans and headings**, through four arms:
//!
//! - **T1** — a table row whose first cell is a ballot id: any bold span in the remaining cells
//!   carries a marker → that id is claimed open;
//! - **T2** — a list item headed `- **<id>`: any bold span in the item (the item absorbs its
//!   indented continuation lines) carries a marker → that id is claimed open;
//! - **T3** — any block: a bold span that carries a marker claims every id named *inside that
//!   span*;
//! - **T4** — a heading that carries a marker claims every id in it.
//!
//! A bold span is an odd-indexed segment of `split("**")`, **including a trailing unclosed
//! segment**, because the ke16 `:282-283` shape opens its bold on one line and closes it on the
//! next.
//!
//! **Markers**, read only inside bold spans / headings: uppercase whole-word `OPEN` whose next
//! byte is not `-` (so `OPEN-QUESTIONS` is not one), and — case-insensitively — `still open`,
//! `stays open`, `stay open`, `remains open`, `remain open`, `remains unanswered`,
//! `remain unanswered`, `not yet ruled`, `awaiting an answer`. Bare `UNANSWERED` is
//! deliberately NOT a marker: GB-9's own ruled sites say `**Its deadline expired unanswered**`
//! (`OPEN-QUESTIONS.md:75`) and `**The deadline EXPIRED UNANSWERED — R0 landed with GB-9
//! open**` (`:706` after the 2026-09-10 port; `:622` before it), and both are records of a ruled
//! ballot, not claims that it is open.
//!
//! **The narrowing, stated because GB-8 forbids the alternative.** Every register on both
//! branches writes a disposition as a leading bold phrase (`⚠ **OPEN — …**`, `⚠ **STAYS
//! OPEN.**`, `✅ **RULED …**`), and every measured false red is prose *outside* bold — this
//! branch's `CAMPAIGN.md:109` says *"four registers on the working branch still say OPEN"*
//! inside GB-5's own RULED row, `OPEN-QUESTIONS.md:1032` (`:941` before the 2026-09-10 port)
//! says *"while this ballot was OPEN"* inside F7's body, and `OPEN-QUESTIONS.md:489` (`:405`
//! before the port) opens `**Still open: F8, F10 and F9's residual VALUES question**` beside
//! `**GB-5 is RULED**` in the same paragraph. A line-level
//! or paragraph-level predicate reds those; a bold-span predicate catches **three of the four**
//! historic GB-5 register sites — ke16 `OPEN-QUESTIONS.md:250` and `:282`, and
//! `gaia/CAMPAIGN.md:124` — plus the ke16 AB-7 body at `OPEN-QUESTIONS.md:1196`, and none of
//! the six measured false reds. Those four shapes are the ones pinned as fixtures below.
//!
//! **The fourth GB-5 site is the narrowing**, and it is the one shape this predicate does NOT
//! see: prose that says a ruled ballot is open without bold and without the id in the same bold
//! span — ke16 `OPEN-QUESTIONS.md:195-199`, a `### Owner ballots still open` heading (which
//! names no ballot id, so the T4 arm cannot fire on it) over a paragraph whose `**GB-5**` bold
//! span carries no marker of its own. Neither half is a claim by itself; only their adjacency
//! is, and adjacency is exactly what a decidable predicate cannot read here. The failure
//! message prints this narrowing so nobody widens the check with a skip.
//!
//! **NO WAIVERS (GB-8).** Ballot GB-8 was ruled 2026-08-30, delegated: *no per-site waivers,
//! at any census, ever; where a property is not decidable as written, narrow the PREDICATE and
//! print the narrowing in the failure message.* This file carries no skip list, no
//! `<!-- census-ignore -->` marker, no environment override and no `#[ignore]`. A red is fixed
//! in the register, never here.
//!
//! # Scope
//!
//! English files only: `docs/gaia/DECISIONS.md`, `docs/OPEN-QUESTIONS.md`,
//! `docs/gaia/CAMPAIGN.md`. `docs/ru/` is **FROZEN by owner decision, 2026-09-07** — the
//! English documents are the only side maintained — and is neither scanned nor edited here.
//! ⚠ This branch's `CLAUDE.md:156`, `docs/ru/README.md` and `docs/OPEN-QUESTIONS.md:7-8` still
//! print the withdrawn same-commit pairing rule; the freeze outranks them and landing its text
//! on this branch is the orchestrator's, not this file's. `docs/aether-v2/` is out of scope by
//! decision, not oversight: its ballot tables have no `who` column, so the source predicate
//! above does not apply to them, and widening is a separate call. Known stale "open" claims
//! there are recorded in `docs/OPEN-QUESTIONS.md` §*2026-09-10*.
//!
//! # On the merge tree `merge/ke16-into-render`, 2026-09-10 — what still holds and what does NOT
//!
//! This file was written on `docs/ab-register-sync` and is landed here unchanged below this
//! section: not one constant, predicate or fixture is different. What follows is the currency
//! statement its own *Scope* section owes a reader on a different tree.
//!
//! **The scope statement holds.** All three scanned paths exist here — `docs/gaia/DECISIONS.md`,
//! `docs/OPEN-QUESTIONS.md`, `docs/gaia/CAMPAIGN.md`. `docs/ru/` is neither scanned nor edited
//! (FROZEN, owner, 2026-09-07). `docs/aether-v2/` is still out of scope for the same structural
//! reason: its ballot tables carry no `who` column. The ⚠ note about the withdrawn same-commit
//! pairing rule was re-checked here and is still accurate — `CLAUDE.md:156` and
//! `docs/OPEN-QUESTIONS.md:7-8` on this tree both still print it, and landing the freeze text is
//! the orchestrator's, not this file's.
//!
//! **But the merge changed both target files, so the pasted runs below do not reproduce here, and
//! that is said rather than papered over.** The merge took `feat/threadpool-ke16`'s registers into
//! `docs/OPEN-QUESTIONS.md` and `docs/gaia/CAMPAIGN.md` under a union rule, which re-imported that
//! branch's PRE-RULING text. Live on this tree, after the repair that made this gate green:
//! `ruled ids: 16 (26 sites)`, `open claims: 5 ids (23 sites; T1/T2/T3/T4 = 6/3/14/0)`,
//! `open = [AB-12, AB-7, F10, F8, F9]`, `docs/OPEN-QUESTIONS.md: 980 blocks scanned`,
//! `docs/gaia/CAMPAIGN.md: 99 blocks scanned`. The two ruled GB-5 sites the blocks below name as
//! `docs/gaia/DECISIONS.md:327` and `docs/OPEN-QUESTIONS.md:71` are `:1262` and `:80` here; the
//! T1 mutation site `docs/gaia/CAMPAIGN.md:109` is `:176`. **Read the blocks below as the record
//! of a run on the tree they were taken on.**
//!
//! **`ruled ids` is 16 and not 28, and the reason matters more than the number.** The AB index
//! ported into `OPEN-QUESTIONS.md` §*2026-09-10* on the register lane is NOT in this merge
//! (measured: no `## 2026-09-10` heading in this tree's `docs/OPEN-QUESTIONS.md`). This tree's
//! AB rulings live in the `## 2026-08-30 — the owner's rulings` table at
//! `docs/OPEN-QUESTIONS.md:583-591`, whose header row at `:583` is `| ballot | ruling | where the
//! ground lives |` — **no `who` cell**, so the source predicate does not read it, by construction
//! and not by accident. Consequence, stated because it is load-bearing: the ruled count sits
//! EXACTLY on `MIN_RULED_IDS`, with zero headroom. That is the safe direction — one ruled id lost
//! trips the vacuity guard loudly instead of narrowing the gate silently — but it means the floor
//! is at its bound here and must be raised, not lowered, when the AB index lands. ⚠ And the
//! *Scope* section's closing pointer — "known stale 'open' claims … recorded in
//! `docs/OPEN-QUESTIONS.md` §*2026-09-10*" — is DEAD on this tree for the same reason: that
//! entry is on the register lane and did not come through the merge.
//!
//! ⚠ **A blind spot MEASURED here and deliberately NOT repaired.** `AB-7`'s body at
//! `docs/OPEN-QUESTIONS.md:1773` reads `**STILL OPEN — STILL THE OWNER'S.**` while `:587` in the
//! table above records the owner lifting it. This census reports `AB-7` as open and does **not**
//! red, because the `who`-less table at `:583` is not a ruling source — the same structural gap
//! as the paragraph above. Widening the source predicate to that table would also move what the
//! vacuity floor means, so it is a separate call and is recorded, not taken.
//!
//! **The narrowing this header describes is present verbatim on this tree.** ke16's
//! `### Owner ballots still open` heading (`:195-199` there) landed at
//! `docs/OPEN-QUESTIONS.md:566`, over a paragraph naming `**GB-5**` and `**GB-6**`. The predicate
//! did not see it and could not; it was struck and dated by hand in the same pass, exactly as the
//! *NO WAIVERS* rule prescribes — the register was fixed, nothing here was widened or skipped.
//!
//! # Home
//!
//! The workspace-root package (`boyko-engine`), for the reason
//! [`internal_docs_anchors.rs`](internal_docs_anchors.rs) gives first: `CARGO_MANIFEST_DIR`
//! **is** the repository root here, so no `../..` walking can silently aim the scan at the
//! wrong tree. Hand-rolled scanning follows from the same home — the root package pulls in no
//! regex crate to reach for, and this census compiles against `std` alone.
//!
//! ⚠ **Its second reason is stale, and is NOT repeated here.** `internal_docs_anchors.rs:36-38`
//! says the root package "has zero dependencies, so the gate needs no GPU, no `dxc`, no golden
//! corpus and no build of the engine". On this branch `Cargo.toml:79` still has an empty
//! `[dependencies]` table, but `:81-87` declares one **dev**-dependency —
//! `boyko-diag = { path = "crates/boyko_diag" }`, added for `engine_packages_census` to pin the
//! constant it checks. `boyko-diag` is the bottom of the workspace graph and keeps its own
//! `[dependencies]` table empty, so the edge adds no third-party code and no GPU requirement —
//! but it does mean an engine crate **is** compiled under `-p boyko-engine --all-targets`, and
//! a lint red inside `boyko_diag` fails that command even though nothing in this file changed.
//! Measured while writing this: `cargo clippy -p boyko-engine --all-targets -- -D warnings`
//! reds on `crates/boyko_diag/src/lane.rs:136` with `initializer for 'thread_local' value can be
//! made 'const'` — a false positive, and demonstrably one, because `lane.rs:139` *already* reads
//! `static LANE: Cell<u16> = const { Cell::new(LANE_UNCLAIMED) };`. It is pre-existing (that
//! crate is untouched by this lane) and belongs to whoever lands the crate-level `allow`; the
//! same command without `-D warnings` reports that one warning and **nothing** attributable to
//! `boyko-engine` or to this test target. Repairing the stale sentence in
//! `internal_docs_anchors.rs` is out of this lane's scope; restating it as present-tense fact
//! here would have been the doc-rot this repository catalogues, so the accurate statement is
//! written above instead.
//!
//! # Red-first proof, by mutation
//!
//! The live counts are the ones the run prints under `--nocapture`; the header does not restate
//! them. Every mutation below was performed on a working tree whose `CAMPAIGN.md` /
//! `OPEN-QUESTIONS.md` had been snapshotted and `cmp`-proved first, was restored by `cp` from
//! that snapshot, and was `cmp`-proved silent again afterwards. Each block is pasted as the run
//! printed it, with one marked elision: the failure's closing narrowing paragraph, which is the
//! `NARROWING` constant below and is quoted in full under *The predicate, and its narrowing*.
//!
//! ⚠ **The `panicked at tests\gaia_ruled_vs_open_census.rs:NNN` line in the pasted blocks reads
//! `:864`, and that is NOT where the `panic!` stands today.** It is the line the run actually
//! printed, and it names the `panic!` that closes `ruled_ballots_are_not_marked_open` — a site
//! that moves down by exactly as many lines as this header grows. It was `:864` when the blocks
//! were pasted, `:902` on the first shipped revision, and the review corrections moved it again.
//! **No current number is written here on purpose**: pinning one makes the next header edit
//! falsify it, which is how the `:864` in the blocks went stale in the first place — chase the
//! anchor (`grep -n 'panic!("{msg}");'`), not a number. Everything else in the blocks — the
//! sites, the counts, the arm histogram and the message text — is reproducible verbatim on this
//! commit, and the location is the one field that is not.
//!
//! ⚠ **Two of the four runs recorded here are NOT reproducible on this tree, and are marked so
//! rather than dropped.** The gate was first proven red on 2026-09-10 *before* its sibling edit
//! — the AB index ported into `OPEN-QUESTIONS.md` §*2026-09-10* — was inserted. Those runs
//! report `ruled ids: 16` and GB-5's body at `:567`; the port added twelve ruled ids and moved
//! every line below its insertion point down — GB-5's body `:566` → `:650` and F7's `:941` →
//! `:1032`, a shift of +84 above the `AB` bodies and +91 below them, because the eight `RULED`
//! pointer lines land in between — so a reader who re-runs them today gets neither number.
//! They are kept because a red-first record is a record of what was actually run, but the pair
//! under *On the shipped tree* is the proof a reader can reproduce on this commit, and it is
//! the one that gates the file.
//!
//! ## On the shipped tree — the reproducible proof, both arms
//!
//! **Mutation T1** — `docs/gaia/CAMPAIGN.md:109`, GB-5's disposition cell: its leading
//! `✅ **RULED BY THE OWNER (recorded 2026-08-31, b6c41237) — PERMIT AS SEED.**` replaced by
//! the ke16 `docs/gaia/CAMPAIGN.md:124` text `⚠ **OPEN — the owner asked for the trade-offs,
//! not a ruling, and a ruling here would be a defect.**`, the rest of the cell untouched:
//!
//! ```text
//! running 3 tests
//! test scanner_rejects_the_known_false_red_shapes ... ok
//! test scanner_catches_the_historic_ke16_shapes ... ok
//! gaia_ruled_vs_open_census: ruled ids: 28 (30 sites)
//! gaia_ruled_vs_open_census: open claims: 5 ids (18 sites; T1/T2/T3/T4 = 6/2/10/0)
//! gaia_ruled_vs_open_census: ruled = [AB-1, AB-10, AB-11, AB-13, AB-2, AB-3, AB-4, AB-5, AB-6, AB-7, AB-8, AB-9, F1, F2, F3, F4, F5, F6, F7, GB-1, GB-2, GB-3, GB-4, GB-5, GB-6, GB-7, GB-8, GB-9]
//! gaia_ruled_vs_open_census: open  = [AB-12, F10, F8, F9, GB-5]
//! gaia_ruled_vs_open_census: docs/OPEN-QUESTIONS.md: 840 blocks scanned
//! gaia_ruled_vs_open_census: docs/gaia/CAMPAIGN.md: 52 blocks scanned
//!
//! thread 'ruled_ballots_are_not_marked_open' (15284) panicked at tests\gaia_ruled_vs_open_census.rs:864:5:
//!
//! RULED ballots claimed OPEN in a register (1):
//!   GB-5
//!     ruled at:
//!       docs/gaia/DECISIONS.md:327 [heading carrying RULED] «## GB-5 — RULED BY THE OWNER, 2026-08-30: **permit as SEED**»
//!       docs/OPEN-QUESTIONS.md:71 [who-table row, who = `owner`] «| **GB-5** | owner | **PERMIT AS SEED** — ruled 2026-08-30, recorded 2026-08-31. ⚠ **See the GB-5 section belo…»
//!     claimed OPEN at:
//!       docs/gaia/CAMPAIGN.md:109 [T1 table row, marker `OPEN`] «OPEN — the owner asked for the trade-offs, not a ruling, and a ruling here would be a defect.»
//!
//! [… the narrowing paragraph — the `NARROWING` constant, quoted in full above …]
//!
//! test ruled_ballots_are_not_marked_open ... FAILED
//! test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.12s
//! ```
//!
//! **Mutation T2** — `  ⚠ **STAYS OPEN.**` appended after `docs/OPEN-QUESTIONS.md:650`, the
//! last line of the GB-5 body, so the claim lands at `:651`. This arm is run because T1 alone
//! would leave the T2 (list-item) arm unproven, and GB-5 was lost in bodies as well as rows:
//!
//! ```text
//! running 3 tests
//! gaia_ruled_vs_open_census: ruled ids: 28 (30 sites)
//! gaia_ruled_vs_open_census: open claims: 5 ids (18 sites; T1/T2/T3/T4 = 5/3/10/0)
//! gaia_ruled_vs_open_census: ruled = [AB-1, AB-10, AB-11, AB-13, AB-2, AB-3, AB-4, AB-5, AB-6, AB-7, AB-8, AB-9, F1, F2, F3, F4, F5, F6, F7, GB-1, GB-2, GB-3, GB-4, GB-5, GB-6, GB-7, GB-8, GB-9]
//! gaia_ruled_vs_open_census: open  = [AB-12, F10, F8, F9, GB-5]
//! gaia_ruled_vs_open_census: docs/OPEN-QUESTIONS.md: 840 blocks scanned
//! gaia_ruled_vs_open_census: docs/gaia/CAMPAIGN.md: 52 blocks scanned
//!
//! thread 'ruled_ballots_are_not_marked_open' (17672) panicked at tests\gaia_ruled_vs_open_census.rs:864:5:
//!
//! RULED ballots claimed OPEN in a register (1):
//!   GB-5
//!     ruled at:
//!       docs/gaia/DECISIONS.md:327 [heading carrying RULED] «## GB-5 — RULED BY THE OWNER, 2026-08-30: **permit as SEED**»
//!       docs/OPEN-QUESTIONS.md:71 [who-table row, who = `owner`] «| **GB-5** | owner | **PERMIT AS SEED** — ruled 2026-08-30, recorded 2026-08-31. ⚠ **See the GB-5 section belo…»
//!     claimed OPEN at:
//!       docs/OPEN-QUESTIONS.md:651 [T2 list item `- **GB-5`, marker `OPEN`] «STAYS OPEN.»
//!
//! [… the narrowing paragraph …]
//!
//! test scanner_rejects_the_known_false_red_shapes ... ok
//! test scanner_catches_the_historic_ke16_shapes ... ok
//! test ruled_ballots_are_not_marked_open ... FAILED
//! test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.12s
//! ```
//!
//! Both restored by `cp` from the snapshot, `cmp` silent both times, `git diff --stat` back to
//! the shipped delta, re-run green: `running 3 tests … 3 passed`, `ruled ids: 28 (30 sites)`,
//! `open claims: 4 ids (17 sites; T1/T2/T3/T4 = 5/2/10/0)`, `open = [AB-12, F10, F8, F9]` —
//! every one of those four genuinely open, and none of them in the ruled set.
//!
//! ## The T4 arm — a red on the SCANNER, because the corpus cannot produce one
//!
//! The two mutations above mutate a *register* and prove the gate. They cannot reach the T4
//! (heading) arm, because no register on either branch writes a heading that both carries a
//! marker and names a ballot id: the live run prints `T4 = 0`, and it printed `T4 = 0` on the
//! first shipped revision too. An arm at zero on the corpus and unpinned by any fixture is an
//! arm whose regression is invisible to everything committed — the "check that could not fail"
//! shape this repository catalogues, one layer in. So the arm now carries the synthetic pin in
//! `scanner_catches_the_historic_ke16_shapes`, and the pin was proven red by mutating the
//! **scanner** instead of a document: `open_claims`'s heading arm, `for id in
//! ballot_ids(&b.text)` → `for id in ballot_ids("")`, one line, restored by `cp` from a
//! `cmp`-proved snapshot and `cmp`-proved silent afterwards.
//!
//! ```text
//! running 3 tests
//! test scanner_catches_the_historic_ke16_shapes ... FAILED
//! test scanner_rejects_the_known_false_red_shapes ... ok
//! test ruled_ballots_are_not_marked_open ... ok
//!
//! ---- scanner_catches_the_historic_ke16_shapes stdout ----
//! assertion `left == right` failed: a marked heading claims its own ids and only those: {} (arms ArmCounts { t1: 0, t2: 0, t3: 0, t4: 0 })
//!   left: {}
//!  right: {"AB-6", "F3"}
//!
//! test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
//! ```
//!
//! Read the third line: `ruled_ballots_are_not_marked_open ... ok`. **The corpus gate stayed
//! green through a mutation that deleted a whole arm of its own predicate**, which is the
//! measurement, not a footnote — before this pin, the T4 arm could have been broken (or removed)
//! in any commit and every committed check would still have reported green.
//!
//! ## Before the port — the first red, kept for the record, NOT reproducible here
//!
//! Same two mutations, run when `OPEN-QUESTIONS.md` still ended its 2026-09-03 entry at `:83`
//! and the ruled set was the 16 `F`/`GB` ids. T1 (`docs/gaia/CAMPAIGN.md:109`) and T2
//! (`docs/OPEN-QUESTIONS.md:567`, GB-5's body at its pre-port numbering) both red, naming the
//! same two ruled sites — `docs/gaia/DECISIONS.md:327` and `docs/OPEN-QUESTIONS.md:71`, whose
//! line numbers the port did not move — against `ruled ids: 16 (17 sites)` and
//! `open = [F10, F8, F9, GB-5]`. The T1 run of that pair reported `1 passed; 2 failed`: its
//! second failure was this file's own, not the register's — the ke16-shapes fixture had been
//! written with `\n\` string continuations, which un-indent a bullet's body, so its AB-7 pin
//! returned no claim. The fixtures have been raw strings since, and the gate on the shipped
//! tree is `3 passed`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

/// The decision log whose RULED headings are a source of rulings.
const DECISIONS: &str = "docs/gaia/DECISIONS.md";
/// The register whose `who`-tables are a source of rulings AND whose bodies are a target.
const OPEN_QUESTIONS: &str = "docs/OPEN-QUESTIONS.md";
/// The campaign file: a target only. Its ballot table has no `who` column and is not a source.
const CAMPAIGN: &str = "docs/gaia/CAMPAIGN.md";

/// Files whose bold spans and headings are read for OPEN claims.
const OPEN_TARGETS: &[&str] = &[OPEN_QUESTIONS, CAMPAIGN];

/// `who` cell prefixes that mean "ruled". Closed vocabulary; printed in the failure message.
const WHO_RULED_PREFIXES: &[&str] = &["owner", "delegated", "standing rule"];
/// `who` cell substrings that veto "ruled" even when a prefix matches.
const WHO_NOT_RULED: &[&str] = &["partly", "asked", "analys"];
/// Uppercase whole words that make a DECISIONS heading a ruling.
const RULED_HEADING_WORDS: &[&str] = &["RULED", "RESOLVED", "DISPOSED", "RATIFIED"];
/// Case-insensitive phrase markers of an OPEN claim (beside uppercase whole-word `OPEN`).
const OPEN_PHRASES: &[&str] = &[
    "still open",
    "stays open",
    "stay open",
    "remains open",
    "remain open",
    "remains unanswered",
    "remain unanswered",
    "not yet ruled",
    "awaiting an answer",
];

/// Floor on the ruled-id count. Measured 16 on this branch before the AB index was ported
/// (F1-F7, GB-1..GB-9); a run below it means a source shape stopped being read.
const MIN_RULED_IDS: usize = 16;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

// ───────────────────────────── ballot id grammar ─────────────────────────────

fn is_id_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Every ballot id in `text`, in order of appearance: `F1..F10`, `GB-1..GB-9`, `AB-1..AB-13`.
/// A prefix must sit at a boundary (previous byte not `[A-Za-z0-9_-]`), the digit run must end
/// at a boundary, and out-of-range numbers are ignored.
fn ballot_ids(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let boundary_before = i == 0 || !is_id_byte(bytes[i - 1]);
        if boundary_before {
            for (prefix, max) in [("F", 10usize), ("GB-", 9), ("AB-", 13)] {
                let p = prefix.as_bytes();
                if bytes[i..].starts_with(p) {
                    let mut j = i + p.len();
                    let digits_start = j;
                    while j < bytes.len() && bytes[j].is_ascii_digit() {
                        j += 1;
                    }
                    let boundary_after = j == bytes.len() || !is_id_byte(bytes[j]);
                    if j > digits_start && boundary_after {
                        let n: usize = text[digits_start..j].parse().unwrap_or(0);
                        if (1..=max).contains(&n) && !text[digits_start..j].starts_with('0') {
                            out.push(format!("{prefix}{n}"));
                            i = j;
                            break;
                        }
                    }
                }
            }
        }
        i += 1;
    }
    out
}

// ───────────────────────────── markers ─────────────────────────────

/// Uppercase whole-word `word` occurs in `text`; a following `-` disqualifies (so `OPEN` does
/// not match `OPEN-QUESTIONS`), a preceding word byte disqualifies (so `REOPEN` does not).
fn has_upper_word(text: &str, word: &str) -> bool {
    let bytes = text.as_bytes();
    let w = word.as_bytes();
    let mut from = 0;
    while let Some(pos) = text[from..].find(word) {
        let start = from + pos;
        let end = start + w.len();
        let before_ok = start == 0 || !is_word_byte(bytes[start - 1]);
        let after_ok = end == bytes.len() || (!is_word_byte(bytes[end]) && bytes[end] != b'-');
        if before_ok && after_ok {
            return true;
        }
        from = start + 1;
    }
    false
}

/// The marker a bold span / heading carries, if any.
fn open_marker(text: &str) -> Option<&'static str> {
    if has_upper_word(text, "OPEN") {
        return Some("OPEN");
    }
    let lower = text.to_ascii_lowercase();
    OPEN_PHRASES.iter().copied().find(|p| lower.contains(p))
}

fn ruled_heading_word(text: &str) -> Option<&'static str> {
    RULED_HEADING_WORDS.iter().copied().find(|w| has_upper_word(text, w))
}

// ───────────────────────────── blocks ─────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Heading,
    TableRow,
    ListItem,
    Paragraph,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Heading => "heading",
            Kind::TableRow => "table row",
            Kind::ListItem => "list item",
            Kind::Paragraph => "paragraph",
        }
    }
}

struct Block {
    kind: Kind,
    /// 1-based line the block starts on.
    line: usize,
    /// The block's lines joined with a single space, `>` prefixes stripped.
    text: String,
    /// `(byte offset into text, 1-based line number)` for every source line of the block.
    line_starts: Vec<(usize, usize)>,
    /// `TableRow` only: the row's cells, escape-aware (`\|` is not a separator).
    cells: Vec<String>,
    /// `TableRow` only: the header cells of the table this row belongs to, when the table has a
    /// header (a `|` line immediately followed by a `---` separator row).
    header: Option<Vec<String>>,
}

impl Block {
    fn line_of(&self, offset: usize) -> usize {
        let mut line = self.line;
        for &(start, ln) in &self.line_starts {
            if start <= offset {
                line = ln;
            } else {
                break;
            }
        }
        line
    }

    /// Bold spans as `(start offset in text, span text)`: odd-indexed segments of a `**` split,
    /// including a trailing unclosed one.
    fn bold_spans(&self) -> Vec<(usize, &str)> {
        bold_spans_of(&self.text)
    }
}

fn bold_spans_of(text: &str) -> Vec<(usize, &str)> {
    let mut spans = Vec::new();
    let mut offset = 0;
    for (idx, seg) in text.split("**").enumerate() {
        if idx % 2 == 1 {
            spans.push((offset, seg));
        }
        offset += seg.len() + 2;
    }
    spans
}

fn strip_quote(line: &str) -> &str {
    let mut s = line.trim_start();
    while let Some(rest) = s.strip_prefix('>') {
        s = rest.trim_start();
    }
    s
}

/// Escape-aware cell split of a `|` line: `\|` stays inside its cell.
fn split_cells(line: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(&next) = chars.peek() {
                    cur.push(c);
                    cur.push(next);
                    chars.next();
                } else {
                    cur.push(c);
                }
            }
            '|' => {
                cells.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    cells.push(cur);
    // A row `| a | b |` splits into ["", " a ", " b ", ""]: drop the empty ends.
    if cells.first().is_some_and(|c| c.trim().is_empty()) {
        cells.remove(0);
    }
    if cells.last().is_some_and(|c| c.trim().is_empty()) {
        cells.pop();
    }
    cells.into_iter().map(|c| c.trim().to_string()).collect()
}

fn is_separator_row(cells: &[String]) -> bool {
    !cells.is_empty()
        && cells
            .iter()
            .all(|c| !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':' || ch == ' '))
}

fn is_list_start(line: &str) -> bool {
    line.starts_with("- ") || line.starts_with("* ")
}

fn is_fence(line: &str) -> bool {
    line.trim_start().starts_with("```")
}

/// Segment a markdown file into blocks. Fenced code is skipped. Headings and table rows are one
/// block per line; a list item absorbs every following blank / indented / `>` line until the
/// next column-0 non-blank line; everything else is a paragraph of consecutive non-blank lines.
fn segment(text: &str) -> Vec<Block> {
    let lines: Vec<&str> = text.lines().map(|l| l.trim_end_matches('\r')).collect();
    let mut blocks: Vec<Block> = Vec::new();
    let mut in_fence = false;
    let mut table_header: Option<Vec<String>> = None;
    let mut prev_was_table = false;
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let ln = i + 1;
        if is_fence(line) {
            in_fence = !in_fence;
            prev_was_table = false;
            i += 1;
            continue;
        }
        if in_fence {
            i += 1;
            continue;
        }
        if line.trim().is_empty() {
            prev_was_table = false;
            i += 1;
            continue;
        }
        if line.starts_with('#') {
            prev_was_table = false;
            blocks.push(Block {
                kind: Kind::Heading,
                line: ln,
                text: line.to_string(),
                line_starts: vec![(0, ln)],
                cells: Vec::new(),
                header: None,
            });
            i += 1;
            continue;
        }
        if line.starts_with('|') {
            if !prev_was_table {
                table_header = None;
            }
            prev_was_table = true;
            let cells = split_cells(line);
            if is_separator_row(&cells) {
                // The row pushed just before is this table's header: lift it out.
                if let Some(header) =
                    blocks.pop_if(|b| b.kind == Kind::TableRow && b.line + 1 == ln)
                {
                    table_header = Some(header.cells);
                }
                i += 1;
                continue;
            }
            blocks.push(Block {
                kind: Kind::TableRow,
                line: ln,
                text: line.to_string(),
                line_starts: vec![(0, ln)],
                cells,
                header: table_header.clone(),
            });
            i += 1;
            continue;
        }
        prev_was_table = false;
        if is_list_start(line) {
            let mut text = String::new();
            let mut line_starts = Vec::new();
            let mut j = i;
            loop {
                let l = lines[j];
                if !l.trim().is_empty() {
                    if !text.is_empty() {
                        text.push(' ');
                    }
                    line_starts.push((text.len(), j + 1));
                    text.push_str(strip_quote(l));
                }
                j += 1;
                if j >= lines.len() {
                    break;
                }
                let next = lines[j];
                let continues = next.trim().is_empty()
                    || next.starts_with(' ')
                    || next.starts_with('\t')
                    || next.starts_with('>');
                if !continues || is_fence(next) {
                    break;
                }
            }
            blocks.push(Block {
                kind: Kind::ListItem,
                line: ln,
                text,
                line_starts,
                cells: Vec::new(),
                header: None,
            });
            i = j;
            continue;
        }
        // Paragraph.
        let mut text = String::new();
        let mut line_starts = Vec::new();
        let mut j = i;
        while j < lines.len() {
            let l = lines[j];
            if l.trim().is_empty()
                || l.starts_with('#')
                || l.starts_with('|')
                || is_list_start(l)
                || is_fence(l)
            {
                break;
            }
            if !text.is_empty() {
                text.push(' ');
            }
            line_starts.push((text.len(), j + 1));
            text.push_str(strip_quote(l));
            j += 1;
        }
        blocks.push(Block {
            kind: Kind::Paragraph,
            line: ln,
            text,
            line_starts,
            cells: Vec::new(),
            header: None,
        });
        i = j;
    }
    blocks
}

// ───────────────────────────── the two sets ─────────────────────────────

/// A site in a file, for the failure message.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Site {
    file: String,
    line: usize,
    /// Which arm / source shape produced it.
    how: String,
    /// A short excerpt, for the failure message.
    excerpt: String,
}

impl Site {
    fn render(&self) -> String {
        format!("{}:{} [{}] «{}»", self.file, self.line, self.how, self.excerpt)
    }
}

fn excerpt(text: &str) -> String {
    let t: String = text.chars().take(110).collect();
    if t.len() < text.len() { format!("{t}…") } else { t }
}

fn strip_id_cell(cell: &str) -> String {
    cell.chars().filter(|c| *c != '*' && !c.is_whitespace()).collect()
}

fn is_exact_id(s: &str) -> bool {
    let ids = ballot_ids(s);
    ids.len() == 1 && ids[0] == s
}

/// The `who`-column verdict: `Some(true)` ruled, `Some(false)` not ruled, `None` no `who` cell.
fn who_says_ruled(block: &Block) -> Option<(bool, String)> {
    let header = block.header.as_ref()?;
    let who_idx = header.iter().position(|h| h.trim().eq_ignore_ascii_case("who"))?;
    let raw = block.cells.get(who_idx)?.clone();
    let norm = raw.replace('*', "").trim().to_ascii_lowercase();
    let prefix_ok = WHO_RULED_PREFIXES.iter().any(|p| norm.starts_with(p));
    let vetoed = WHO_NOT_RULED.iter().any(|v| norm.contains(v));
    Some((prefix_ok && !vetoed, raw))
}

type Sites = BTreeMap<String, Vec<Site>>;

fn add(map: &mut Sites, id: &str, site: Site) {
    let v = map.entry(id.to_string()).or_default();
    if !v.contains(&site) {
        v.push(site);
    }
}

/// RULED ids from `docs/gaia/DECISIONS.md`-shaped text: headings carrying a ruling word.
fn ruled_from_headings(file: &str, text: &str) -> Sites {
    let mut out = Sites::new();
    for b in segment(text) {
        if b.kind != Kind::Heading {
            continue;
        }
        if let Some(word) = ruled_heading_word(&b.text) {
            for id in ballot_ids(&b.text) {
                add(
                    &mut out,
                    &id,
                    Site {
                        file: file.to_string(),
                        line: b.line,
                        how: format!("heading carrying {word}"),
                        excerpt: excerpt(&b.text),
                    },
                );
            }
        }
    }
    out
}

/// RULED ids from `docs/OPEN-QUESTIONS.md`-shaped text: rows of `who`-tables.
fn ruled_from_who_tables(file: &str, text: &str) -> Sites {
    let mut out = Sites::new();
    for b in segment(text) {
        if b.kind != Kind::TableRow {
            continue;
        }
        let Some(first) = b.cells.first() else { continue };
        let id = strip_id_cell(first);
        if !is_exact_id(&id) {
            continue;
        }
        if let Some((true, who)) = who_says_ruled(&b) {
            add(
                &mut out,
                &id,
                Site {
                    file: file.to_string(),
                    line: b.line,
                    how: format!("who-table row, who = `{who}`"),
                    excerpt: excerpt(&b.text),
                },
            );
        }
    }
    out
}

/// Per-arm tally, printed with the counts.
#[derive(Default, Debug)]
struct ArmCounts {
    t1: usize,
    t2: usize,
    t3: usize,
    t4: usize,
}

/// OPEN claims in a target file, through the four arms.
fn open_claims(file: &str, text: &str, arms: &mut ArmCounts) -> (Sites, usize) {
    let mut out = Sites::new();
    let blocks = segment(text);
    let scanned = blocks.len();
    for b in &blocks {
        match b.kind {
            Kind::TableRow => {
                if let Some(first) = b.cells.first() {
                    let id = strip_id_cell(first);
                    if is_exact_id(&id) {
                        let rest = b.cells[1..].join(" | ");
                        for (_, span) in bold_spans_of(&rest) {
                            if let Some(m) = open_marker(span) {
                                arms.t1 += 1;
                                add(
                                    &mut out,
                                    &id,
                                    Site {
                                        file: file.to_string(),
                                        line: b.line,
                                        how: format!("T1 table row, marker `{m}`"),
                                        excerpt: excerpt(span),
                                    },
                                );
                            }
                        }
                    }
                }
            }
            Kind::ListItem => {
                let head = b.text.trim_start_matches(['-', '*']).trim_start();
                if let Some(inner) = head.strip_prefix("**") {
                    let ids = ballot_ids(inner);
                    if let Some(id) = ids.first()
                        && inner.starts_with(id.as_str())
                    {
                        for (off, span) in b.bold_spans() {
                            if let Some(m) = open_marker(span) {
                                arms.t2 += 1;
                                add(
                                    &mut out,
                                    id,
                                    Site {
                                        file: file.to_string(),
                                        line: b.line_of(off),
                                        how: format!("T2 list item `- **{id}`, marker `{m}`"),
                                        excerpt: excerpt(span),
                                    },
                                );
                            }
                        }
                    }
                }
            }
            Kind::Heading => {
                if let Some(m) = open_marker(&b.text) {
                    for id in ballot_ids(&b.text) {
                        arms.t4 += 1;
                        add(
                            &mut out,
                            &id,
                            Site {
                                file: file.to_string(),
                                line: b.line,
                                how: format!("T4 heading, marker `{m}`"),
                                excerpt: excerpt(&b.text),
                            },
                        );
                    }
                }
            }
            Kind::Paragraph => {}
        }
        // T3: any block, any bold span that carries a marker claims the ids inside it.
        for (off, span) in b.bold_spans() {
            if let Some(m) = open_marker(span) {
                for id in ballot_ids(span) {
                    arms.t3 += 1;
                    add(
                        &mut out,
                        &id,
                        Site {
                            file: file.to_string(),
                            line: b.line_of(off),
                            how: format!("T3 bold span in a {}, marker `{m}`", b.kind.name()),
                            excerpt: excerpt(span),
                        },
                    );
                }
            }
        }
    }
    (out, scanned)
}

fn merge(into: &mut Sites, from: Sites) {
    for (id, sites) in from {
        for s in sites {
            add(into, &id, s);
        }
    }
}

const NARROWING: &str = "\
The predicate reads OPEN only from bold spans of a ballot's own row / bullet, bold spans naming \
the id, and headings; prose that says a ruled ballot is open without bold and without the id in \
the same span is NOT seen (ke16 OPEN-QUESTIONS.md:195-199 shape: an id-less `still open` heading \
over a paragraph whose bold `**GB-5**` carries no marker). RULED comes from headings in \
docs/gaia/DECISIONS.md carrying RULED/RESOLVED/DISPOSED/RATIFIED and from docs/OPEN-QUESTIONS.md \
table rows whose `who` cell starts with owner / delegated / standing rule and contains none of \
partly / asked / analys. NO WAIVERS (GB-8): fix the register, do not add a skip.";

// ───────────────────────────── the gate ─────────────────────────────

#[test]
fn ruled_ballots_are_not_marked_open() {
    let texts: BTreeMap<&str, String> = [DECISIONS, OPEN_QUESTIONS, CAMPAIGN]
        .iter()
        .map(|f| (*f, read(f)))
        .collect();

    let mut ruled = ruled_from_headings(DECISIONS, &texts[DECISIONS]);
    merge(&mut ruled, ruled_from_who_tables(OPEN_QUESTIONS, &texts[OPEN_QUESTIONS]));

    let mut arms = ArmCounts::default();
    let mut open = Sites::new();
    let mut scanned = Vec::new();
    for file in OPEN_TARGETS {
        let (claims, n) = open_claims(file, &texts[file], &mut arms);
        scanned.push((*file, n));
        merge(&mut open, claims);
    }

    let ruled_sites: usize = ruled.values().map(Vec::len).sum();
    let open_sites: usize = open.values().map(Vec::len).sum();
    println!("gaia_ruled_vs_open_census: ruled ids: {} ({} sites)", ruled.len(), ruled_sites);
    println!(
        "gaia_ruled_vs_open_census: open claims: {} ids ({} sites; T1/T2/T3/T4 = {}/{}/{}/{})",
        open.len(),
        open_sites,
        arms.t1,
        arms.t2,
        arms.t3,
        arms.t4
    );
    println!(
        "gaia_ruled_vs_open_census: ruled = [{}]",
        ruled.keys().cloned().collect::<Vec<_>>().join(", ")
    );
    println!(
        "gaia_ruled_vs_open_census: open  = [{}]",
        open.keys().cloned().collect::<Vec<_>>().join(", ")
    );
    for (file, n) in &scanned {
        println!("gaia_ruled_vs_open_census: {file}: {n} blocks scanned");
    }

    assert!(
        ruled.len() >= MIN_RULED_IDS,
        "vacuity guard: only {} ruled ids were read (floor {MIN_RULED_IDS}); the two source \
         shapes are RULED/RESOLVED/DISPOSED/RATIFIED headings in {DECISIONS} and rows of \
         `who`-tables in {OPEN_QUESTIONS} whose who cell starts with {WHO_RULED_PREFIXES:?} — \
         one of them stopped being read",
        ruled.len()
    );
    assert!(
        open_sites >= 1,
        "vacuity guard: zero OPEN claims were read from {OPEN_TARGETS:?}; if every ballot is \
         genuinely ruled, lower this guard in the same commit that rules the last one — \
         otherwise the marker scan stopped seeing bold spans"
    );

    let conflicts: BTreeSet<&String> =
        ruled.keys().filter(|id| open.contains_key(*id)).collect();
    if conflicts.is_empty() {
        return;
    }
    let mut msg = format!(
        "\nRULED ballots claimed OPEN in a register ({}):\n",
        conflicts.len()
    );
    for id in &conflicts {
        msg.push_str(&format!("  {id}\n    ruled at:\n"));
        for s in &ruled[*id] {
            msg.push_str(&format!("      {}\n", s.render()));
        }
        msg.push_str("    claimed OPEN at:\n");
        for s in &open[*id] {
            msg.push_str(&format!("      {}\n", s.render()));
        }
    }
    msg.push('\n');
    msg.push_str(NARROWING);
    msg.push('\n');
    panic!("{msg}");
}

// ───────────────────────────── the scanner's own pins ─────────────────────────────

fn ids_of(map: &Sites) -> BTreeSet<String> {
    map.keys().cloned().collect()
}

fn set(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|s| s.to_string()).collect()
}

/// The four verbatim shapes that lost GB-5 (and the AB-7 body) on `feat/threadpool-ke16` —
/// firing the T1, T2 and T3 arms — plus one **synthetic** heading that fires T4, the only arm
/// the live corpus leaves at zero. The synthetic pin is marked as such at its site.
#[test]
fn scanner_catches_the_historic_ke16_shapes() {
    // ke16 docs/gaia/CAMPAIGN.md:124 — a T1 table row.
    // Fixtures are raw strings with real newlines: a `\n\` continuation strips the next line's
    // leading whitespace, which silently un-indents a bullet's body and ends the list item —
    // measured on this file's first run, where the AB-7 pin below returned no claim.
    let campaign_124 = r#"| ballot | question | disposition | blocks |
|---|---|---|---|
| **GB-5** | May a scene document declare an ENGINE-DERIVED field (`PointLight.position`)? | ⚠ **OPEN — the owner asked for the trade-offs, not a ruling, and a ruling here would be a defect.** The **analysis** is attached to the ballot body ([`../OPEN-QUESTIONS.md`](../OPEN-QUESTIONS.md) §2026-08-29). What it settles is only this: **the per-field-flag mechanism the ballot assumes is refuted** — all **12** measured engine-derived field members are *conditionally* derived, including `Transform.translation`, and **both** dispositions are pinned by green committed tests today. Four options (refuse / permit / permit-as-seed / refuse-conditionally) are each stated in their strongest form with prices | **the G1 table freeze** |
"#;
    let mut arms = ArmCounts::default();
    let (claims, _) = open_claims("fixture", campaign_124, &mut arms);
    assert_eq!(ids_of(&claims), set(&["GB-5"]), "ke16 CAMPAIGN.md:124 row: {claims:?}");
    assert!(arms.t1 >= 1, "the :124 row must fire the T1 arm, got {arms:?}");

    // ke16 docs/OPEN-QUESTIONS.md:250 — a who-table row whose who cell is
    // `**owner asked for ANALYSIS**`: claimed open by T1, and NOT counted ruled.
    let oq_250 = r#"| ballot | who | ruling | where the ground lives |
|---|---|---|---|
| **GB-5** | **owner asked for ANALYSIS** | ⚠ **STAYS OPEN.** The analysis is attached to the ballot body below. It settles one thing only, and it is a refutation of the ballot's own mechanism: **all 12 measured engine-derived field members are *conditionally* derived**, and both dispositions are pinned by green committed tests today | §2026-08-29, the GB-5 body |
"#;
    let mut arms = ArmCounts::default();
    let (claims, _) = open_claims("fixture", oq_250, &mut arms);
    assert_eq!(ids_of(&claims), set(&["GB-5"]), "ke16 OPEN-QUESTIONS.md:250 row: {claims:?}");
    let ruled = ruled_from_who_tables("fixture", oq_250);
    assert!(
        ruled.is_empty(),
        "a row whose who cell says `asked for ANALYSIS` must not count as ruled: {ruled:?}"
    );

    // ke16 docs/OPEN-QUESTIONS.md:282-283 — a blockquote bold that opens on one line and closes
    // on the next, naming four ids.
    let oq_282 = r#"> ruled: **F2, F4, GB-4**, and **F9 in part**. Sent back for **analysis, not decision**: **GB-5**
> and **GB-6** — both stay OPEN, and the analysis is attached to each body below.
> **Still open and genuinely awaiting an answer: `F8`, `F10`, `GB-5`, `GB-6`, and F9's residual
> VALUES question** (may a document set a flag on an individual authored object). Plus **AB-12**,
> which blocks nothing. The per-ballot index is the table in
"#;
    let mut arms = ArmCounts::default();
    let (claims, _) = open_claims("fixture", oq_282, &mut arms);
    // F9 is claimed too: the span's own text names "F9's residual VALUES question", and a
    // bold span claims every id inside it. (The first draft of this pin expected four ids and
    // was refuted by the scanner on its first run — the fixture is verbatim, the pin was not.)
    assert_eq!(
        ids_of(&claims),
        set(&["F8", "F10", "GB-5", "GB-6", "F9"]),
        "ke16 OPEN-QUESTIONS.md:282-283 blockquote: {claims:?}"
    );
    assert!(arms.t3 >= 5, "the two-line bold must fire T3 for each id, got {arms:?}");
    let gb5_line = claims["GB-5"][0].line;
    assert_eq!(gb5_line, 3, "the claim is reported at the line the bold span opens on");

    // ke16 docs/OPEN-QUESTIONS.md:1194-1196 — the AB-7 bullet whose body says STILL OPEN while the
    // index on the same branch says the owner ruled it.
    let oq_1196 = r#"- **AB-7** — R-DENSE: unconditional with a driver-independent ground that must be ESTABLISHED rather
  than asserted, or lifted by `publish tracked`. ⚠ **re-grounds a ratified refusal**. Blocks **R5**.
  **STILL OPEN — STILL THE OWNER'S.** What was delegated was not the choice but the *measurement*
  underneath it, and it was taken on **2026-08-30**: **the candidate driver-independent ground is
  REFUTED, on both of its conjuncts.**
"#;
    let mut arms = ArmCounts::default();
    let (claims, _) = open_claims("fixture", oq_1196, &mut arms);
    assert_eq!(ids_of(&claims), set(&["AB-7"]), "ke16 OPEN-QUESTIONS.md:1196 bullet: {claims:?}");
    assert!(arms.t2 >= 1, "the bullet must fire the T2 arm, got {arms:?}");
    assert_eq!(claims["AB-7"][0].line, 3, "reported at the line of the STILL OPEN span");

    // The T4 arm, and it is the ONE pin here that is not a historic shape. No register on either
    // branch writes a heading that both carries a marker and names its ballots, so the live
    // corpus reports `T4 = 0` — which means that without this pin the arm would be exercised by
    // nothing at all, and a regression in it would be invisible on the corpus AND on the
    // fixtures. It is live code rather than dead code because ke16's `### Owner ballots still
    // open` (see *the narrowing* in the header) is exactly one ballot id short of firing it: a
    // register that writes its ids into such a heading is caught, and that is the shape this
    // pin holds the arm to. The body line below is the arm's other half — a heading claims the
    // ids IN THE HEADING, never the ids in the prose it stands over, so `GB-1` must not appear.
    let t4_heading = r#"### AB-6 and F3 — STILL OPEN

Body prose naming GB-1, which the heading above must NOT claim: no bold span, no marker.
"#;
    let mut arms = ArmCounts::default();
    let (claims, _) = open_claims("fixture", t4_heading, &mut arms);
    assert_eq!(
        ids_of(&claims),
        set(&["AB-6", "F3"]),
        "a marked heading claims its own ids and only those: {claims:?} (arms {arms:?})"
    );
    assert_eq!(arms.t4, 2, "the heading must fire the T4 arm once per id, got {arms:?}");
    assert_eq!(arms.t1 + arms.t2 + arms.t3, 0, "no other arm may fire on a heading: {arms:?}");
    assert_eq!(claims["AB-6"][0].line, 1, "reported at the heading's own line");
}

/// The measured false-red shapes on this branch: every one of them says "open" next to a ruled
/// id, and every one of them must NOT be read as a claim.
#[test]
fn scanner_rejects_the_known_false_red_shapes() {
    let fixture = r#"| ballot | who | ruling |
|---|---|---|
| **GB-3** | owner (adoption) + delegated (the rewrite) | **The THIRD reference kind is ADOPTED** |
| **GB-5** | owner | **PERMIT AS SEED** — ruled 2026-08-30, recorded 2026-08-31. ⚠ **See the GB-5 section below: this is the one answer the register lost outright, and four registers on the working branch itself still say it is open** |
| **GB-9** | standing rule | **RESOLVED — option (b): the `Or<(Changed<A>, Changed<B>)>`-over-dense emission ban is DELETED, with a record.** ⚠ **Its deadline expired unanswered** — R0 landed with GB-9 open |
| **F9** | delegated, **partly** | ⚠ **Three eliminations RULED, the residual escalated back.** ⚠ **STILL THE OWNER'S:** may a document set a flag on an **individual authored object**? |

| Ballot | Question | Disposition (2026-08-30) | Blocks |
|---|---|---|---|
| **GB-5** | May a scene document declare an ENGINE-DERIVED field (`PointLight.position`)? | ✅ **RULED BY THE OWNER (recorded 2026-08-31, `b6c41237`) — PERMIT AS SEED.** *"the third is the most logical"* The disposition column **must not be a boolean** (all twelve measured members are conditionally derived) — it records the CONDITION and the WRITER. ⚠ The ruling commit touched `DECISIONS.md` and nothing else, which is why four registers on the working branch still say OPEN | **the G1 table freeze — UNBLOCKED** |

| Rung | What | Gate |
|---|---|---|
| **G3** | Identity: asset/object ids. ✅ **GB-3 RULED 2026-08-30 — G3 carries no open ballot**, and gains the **asset-ref dangle check** from it. ⚠ **F8** (name-vs-id) remains the candidate ballot for this rung, and is still open | red fixtures |

> ✅ **STATUS, 2026-09-03 — read this before any ballot body below.** Of the
> fourteen Gaia ballots, **twelve are answered**. **Still open: F8, F10 and F9's residual VALUES
> question** — with two corrections: **GB-5 is RULED** (permit as seed, 2026-08-30) and only the
> registers said otherwise, and **GB-6 was DISPOSED 2026-09-03** in both halves.

- **GB-9** — does the `Or`-over-dense generated-code ban survive the kernel fix?
  ✅ **RESOLVED 2026-08-30 by STANDING RULE — option (b): the ban is DELETED WITH A RECORD.**
  ⚠ **The deadline EXPIRED UNANSWERED — R0 landed with GB-9 open**, which is why the ruling
  reconstructs the record from prose rather than from a reproducible failure.
- **F7** — mods.
  > because it is the proximity-settlement shape this list exists to catch: the ratified
  > §Refusals line *"no external-mod pipeline in v1"* was already in
  > [`gaia/DECISIONS.md`](gaia/DECISIONS.md) while this ballot was OPEN — a ratified line
  > answering half of an open ballot.

See **docs/OPEN-QUESTIONS.md §2026-09-03, row GB-3** for the adoption; the file is named
OPEN-QUESTIONS.md and that name is not a marker.
"#;
    // The bullets above must be read as list items with their bodies, not as a bullet head plus
    // stray paragraphs — otherwise the T2 arm is not what this fixture exercises.
    let kinds: Vec<Kind> = segment(fixture).iter().map(|b| b.kind).collect();
    assert_eq!(
        kinds.iter().filter(|k| **k == Kind::ListItem).count(),
        2,
        "fixture must segment into exactly two list items (GB-9, F7): {kinds:?}"
    );
    let ruled = ruled_from_who_tables("fixture", fixture);
    assert_eq!(
        ids_of(&ruled),
        set(&["GB-3", "GB-5", "GB-9"]),
        "F9 (`partly`) must not be ruled; the others must: {ruled:?}"
    );
    let mut arms = ArmCounts::default();
    let (claims, _) = open_claims("fixture", fixture, &mut arms);
    assert_eq!(
        ids_of(&claims),
        set(&["F8", "F10", "F9"]),
        "only the STATUS blockquote's `Still open: F8, F10 and F9's residual` span is a claim; \
         got {claims:?} (arms {arms:?})"
    );
    for id in ruled.keys() {
        assert!(!claims.contains_key(id), "false red on ruled {id}: {:?}", claims[id]);
    }

    // The id grammar and the marker rules the shapes above depend on, pinned individually.
    assert_eq!(
        ballot_ids("F1..F10, F11, F0, GB-9, GB-10, AB-13, AB-14, F9's, **AB-12**, AIR-06, KE14, R-DENSE, GK-4"),
        vec!["F1", "F10", "GB-9", "AB-13", "F9", "AB-12"]
    );
    assert!(is_exact_id("GB-5") && !is_exact_id("GB-5 x") && !is_exact_id("**GB-5**"));
    assert_eq!(strip_id_cell("**GB-5**"), "GB-5");
    assert!(has_upper_word("STAYS OPEN.", "OPEN"));
    assert!(!has_upper_word("OPEN-QUESTIONS.md", "OPEN"));
    assert!(!has_upper_word("REOPENS the grant", "OPEN"));
    assert!(!has_upper_word("still say it is open", "OPEN"));
    assert_eq!(open_marker("Still open: F8"), Some("still open"));
    assert_eq!(open_marker("Its deadline expired unanswered"), None);
    assert_eq!(open_marker("EXPIRED UNANSWERED — R0 landed with GB-9 open"), None);
    assert_eq!(
        split_cells("| a | `extends\\|copy` | c |"),
        vec!["a", "`extends\\|copy`", "c"]
    );
}
