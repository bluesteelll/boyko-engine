//! Gate for the internal navigation docs: every path they cite must exist, and every line
//! anchor — `file.rs:N` or the bare `(N)` of a member table — must land on the definition it
//! claims.
//!
//! ⚠️ **The corpus is TEN documents, not the four this file's history keeps naming.** It gained
//! the six-document gaia/register corpus on 2026-09-10 (see [`GATED_DOCS`]); every "third
//! document" / "fourth document" below is a dated statement about the corpus as it stood then, not
//! about the corpus now. `GATED_DOCS` is the only place that answers "which documents", and
//! [`EVIDENCE_DOCS`] the only place that answers "at what strength" — both are printed by the run.
//!
//! # Why this test exists
//!
//! `CLAUDE.md` mandates [docs/FEATURE_MAP.md] as the **first point of contact** for "where is
//! X?", with `docs/SYSTEMS.md` and `docs/ARCHITECTURE.md` behind it. Nothing in the repository
//! gated them, so they rotted silently: the refactoring campaign turned god-files into
//! directories and other files were renamed or deleted out from under the docs. Measured by
//! pointing the scanner in this file at the pre-repair documents (`git show HEAD:docs/...`
//! against the same source tree): **354 of the 474 anchors it resolves — 75% — did not point at
//! the definition they name**, and 15 of 829 path mentions were dead. The drift runs from a
//! single line (`QueryFilter` 74 → 75) to thousands (`spawn_batch` cited at 2553 in a file of
//! 1917 lines, `EnableStore` 259 → 599), and `create_archetype` was cited at line 484 of
//! `ecs_master.rs` when it had moved to `entity_api.rs:48` — a different file — while the docs'
//! own preamble claimed "line numbers below are verified against the current source". A wrong
//! anchor is worse than no anchor: it sends a reader — human or agent — to a plausible-looking
//! but unrelated line, and the docs assert their own freshness while doing it.
//!
//! Those denominators are not folklore. Both tests print their per-document counts, so
//! `cargo test -p boyko-engine --test internal_docs_anchors -- --nocapture` reports what the gate
//! is actually enforcing — instead of a number someone wrote down once and never re-derived.
//! ⚠️ **This paragraph used to restate that total ("530 anchors and 883 path mentions across the
//! three documents"), and it went stale the moment `GATED_DOCS` gained a fourth document — the
//! defect it was written to denounce, committed one line below the denunciation.** The live
//! figures are the ones the run prints and are deliberately not repeated here. Widening the
//! scanner moves that denominator, which is the point: it stood at 350 while only the suffix form
//! `file.rs:N` was read, and teaching it the bare parenthesised `(N)` form (below) took it to 509
//! on the unchanged documents, reporting 134 violations in one run — concentrated in exactly the
//! member tables that had never been read by anything. Repairing those tables (a `**File:**`
//! header per file, one citation per member) is what carried it the rest of the way to 530.
//!
//! Source files move every commit; the docs do not. Only a mechanical check keeps them honest,
//! so this runs in the ordinary `cargo test --workspace` gate. It lives in the workspace-root
//! package (`boyko-engine`) for two reasons: `CARGO_MANIFEST_DIR` **is** the repository root, so
//! no `../..` walking can silently point the scan at the wrong tree; and that package has zero
//! dependencies, so the gate needs no GPU, no `dxc`, no golden corpus and no build of the engine.
//!
//! # What is checked
//!
//! 1. **Paths** — every markdown link target `](...)` and every bare root-relative mention in
//!    prose or backticks must exist on disk. The root-relative set is [`ROOT_PREFIXES`]; it was
//!    `crates/` alone until the reflection documents showed what an unseen mention actually costs,
//!    which is not a skipped check but a **misbinding** of every anchor behind it.
//! 2. **Line anchors, in both forms the documents write them** — the suffix form `file.rs:N` and
//!    the bare parenthesised `(N)` that member tables use under a sticky **File:** header. Every
//!    citation must be within the cited file, and line N must look like a definition (`fn` /
//!    `struct` / `enum` / `const` / `static` / `impl` / `trait` / `type` / `pub` /
//!    `macro_rules!`, or a `[table]` header for `.toml`).
//! 3. **Identity** — shape alone cannot tell one definition from another; it passes just as
//!    happily on the wrong `pub fn`. Measured: repointing SYSTEMS.md's `enable_store.rs:299` —
//!    the anchor for `swap_remove_bit` — at `:206`, which is a different function
//!    (`EnableColumn::test`), still satisfies the shape test, because `:206` is itself a
//!    `pub(crate) fn`; only the identity clause rejects it, with
//!    ``does not define `swap_remove_bit` ``. So where a line's backticked symbols pair
//!    one-to-one with its anchors, each anchor's line must contain its symbol. This is a partial
//!    check by construction, and the run prints the decomposition per document rather than this
//!    comment carrying it: identity-asserted, shape-only because the prose does not pair
//!    one-to-one, shape-only because the symbol is not *declared* in the cited file, and the
//!    trailing-`~` waiver, which keeps **neither** shape nor identity — `check_anchor` returns at
//!    the waiver branch before `looks_like_definition` ever runs — leaving only "the file exists
//!    and line N is inside it". The waived class is not a rounding error: on the meshlet plan it
//!    is the *majority* of anchors, so "0 stale" on that document means far less than it does on
//!    the navigation docs.
//! 4. **Range coherence** — `N-M` must satisfy `M >= N`, and `M` must be inside the file. The
//!    shape test reads N only, so an end left behind when the start was re-derived is otherwise
//!    invisible. Measured on the live range `enable_store.rs:65-66`: rewriting it to `:65-54`
//!    reports ``ends before it starts``, and to `:65-99999` reports
//!    ``ends past end of file (1278 lines)``, while the first line 65 keeps passing the shape
//!    test in both cases. ⚠️ **The scanner tested `-` before `~`, so the waived spelling `:N~-M`
//!    parsed no tail at all and reached neither assertion.** Measured on the meshlet plan: **34
//!    occurrences are waiver-first and 23 are plain `:N-M`** — so the check was dead on about
//!    three-fifths of that document's ranges, not on all of them. ⚠️ Rev 12's own justification
//!    for this repair said "every one of its ranges" in four texts and that was FALSE; the 23
//!    plain ones parsed a tail and reached both assertions all along. The repair stands on the 34
//!    regardless. Rev 12 accepts the waiver on either side, and two controls below pin both the
//!    capture and the fact that the live corpus exercises it, because a green corpus run cannot
//!    tell "no incoherent range" from "no range parsed".
//! 5. **Both reverse directions** — a `.rs` source citing a `.md` line, and a `.rs` source citing
//!    another `.rs` line. Bounds only, and line-locally bound: a citation binds to a name written
//!    on its own line or it is not checked, because cross-line stickiness is what produced every
//!    misbinding this file has found. The `.rs` → `.rs` half was read by NOTHING until it was
//!    built — the `.md` half filters its targets to `.md` by construction — and it found **12
//!    citations past the end of the file they name**, against three targets that have all shrunk
//!    under them. Those twelve are named in `RS_KNOWN_STALE` rather than repaired, several living
//!    in a crate another lane is rewriting; the list fails on an unlisted violation *and* on a
//!    listed entry that stops reporting, so it cannot rot in either direction.
//! 6. **Non-emptiness, per document** — a mis-typed pattern or a renamed document must fail
//!    loudly rather than vacuously pass over an empty extraction. The counts are asserted *per
//!    document* so a healthy `FEATURE_MAP.md` cannot mask a `SYSTEMS.md` that suddenly yields
//!    nothing.
//!
//! # Rules for the awkward cases (deliberate, not accidental)
//!
//! * **Fenced code blocks are scanned for anchors but not for paths.** A `crates/...` string
//!   inside an example command or an ASCII tree (`├── crates/`, ARCHITECTURE.md) is not an
//!   assertion that such a file exists, so checking it would manufacture failures for text that
//!   makes no claim. A margin note (`// mod.rs:59`) *is* a line claim, though, and it rots
//!   exactly like a prose one — worse, in fact, because nothing was watching: SYSTEMS.md carries
//!   33 of them and when this gate first reached them **30 were stale**, two of them contradicting
//!   corrections made to the same symbols elsewhere in the same document.
//! * **A fenced note's fragment resolves against its section, not against the repository root.**
//!   The notes are written from the reader's position in the tree — `system/system.rs` under a
//!   section whose **Files:** line is `core/system/` — so the fragment is joined onto that base
//!   path and then onto each ancestor of it, and the first join that exists on disk wins. Nothing
//!   is guessed: a fragment landing on no existing file is skipped. A bare `// :59` inherits the
//!   fence's current target. Unlike the anchor binding below, this base survives a sub-heading,
//!   because `### 9.2` inherits the file list of `## 9`; only a new top-level section clears it.
//!   The fence's identity claim is the pseudo-declaration the note sits on, so
//!   `pub enum ObserverKind { … } // mod.rs:70` asserts that line 70 declares `ObserverKind`.
//! * **A symbol must share a line with the anchor that cites it.** Identity pairing is positional
//!   and happens only when a line's symbol count equals its anchor count, so a soft-wrapped
//!   paragraph that leaves `install_storage_kind::<C>` on the line above its own `(:729)` pairs it
//!   with the *previous* anchor instead. The failure is silent in both directions: a symbol
//!   ending one line while its anchor opens the next pairs the anchor with whatever backticked
//!   word happens to sit beside it — which is how `enable_store.rs:219` survived a repair that
//!   re-derived every other anchor in its paragraph. The symbol was `swap_remove_bit` at the end
//!   of one line, the anchor opened the next, and the pairing picked up the ordinary English
//!   `last` from "snapshot `last`'s bit", a name the file does not declare, so identity was
//!   skipped. Three SYSTEMS.md paragraphs have now been rewrapped so each symbol sits beside its
//!   own citation; that is an authoring constraint of these documents, not a heuristic to be
//!   worked around.
//! * **Identity is claimed only against a file that declares the symbol.** A line may pair an
//!   anchor with a name that is not an item in the cited file at all. The run prints how many such
//!   skips each document takes; ⚠️ this comment used to say "exactly five today", a figure measured
//!   over three documents before `GATED_DOCS` gained a fourth — live it is more than twice that,
//!   and the count is not restated here for the same reason no other count in this file is. The
//!   representative case is SYSTEMS.md's
//!   `` / `From<TagId> for ComponentId` (:61) ``, which reduces to the identifier `From`, while
//!   `tags.rs:61` reads `impl From<TagId> for ComponentId {` — a real definition, and one the
//!   shape test accepts on its `impl ` keyword, but nothing in `tags.rs` *declares* an item named
//!   `From`. Claiming identity there would red a correct anchor. The test is therefore on the *declared*
//!   name — what `leading_decl_name` extracts from some line of the file — not on mere presence,
//!   so a parameter called `last` cannot masquerade as a definition of one. Three of the five are
//!   this `From` shape (FEATURE_MAP.md once, SYSTEMS.md twice, against `tags.rs:61` and `:104`);
//!   the other two pair the deferred `commands()` accessor with `observers/mod.rs:98`, and the
//!   Cargo feature name `big_query_table` (`boyko_ecs/Cargo.toml`) with
//!   `query_type_registry.rs:89`.
//! * **The bare `(N)` form is an anchor; `O(1)` is not.** A member table writes one **File:**
//!   header and then a line number per member in bare parentheses — `` `spawn_one::<A>` (582) ``
//!   — so refusing to read that form would leave the densest citations in these documents
//!   ungated, which is exactly where the worst rot was found. The form is separated from ordinary
//!   parenthesised quantities by two structural rules rather than by a unit blacklist. *First*,
//!   the `(` must not follow an identifier byte — that is what makes `O(1)`, `wrapping_add(1)`
//!   and `pool_reserve_rows(0)` argument lists rather than citations. *Second*, the digits must
//!   be followed by `)`, `,` or `/` — closing the group, introducing a parenthetical note
//!   (`(349, diagnostics)`), or continuing a `(a) / (b)` list; a quantity that merely begins with
//!   digits never does, which rejects `(512 B = …)`, `(4096 with …)`, `(1024-bit dedup)`,
//!   `(0%-gate …)`, `(19 members)`, `(12.6)`, `(14a)` and `(0..16 …)`.
//!   Measured over the three documents as they stood when the form was turned on, that pair of
//!   rules accepted 162 parenthesised spans and rejected every quantity; 159 of the 162 had a
//!   resolvable bound file and so entered the gate. Two spot checks that the discriminator still
//!   holds, both re-runnable: rewriting `O(capacity)` to `O(99999)` and `(19 members)` to
//!   `(199999 members)` in FEATURE_MAP.md leaves its denominator at 206 and the suite green —
//!   neither is read as a citation at all — while rewriting a real member ref
//!   `has_component(entity, id)` (673) to `(99999)` reports
//!   `is past end of file (793 lines)`.
//! * **Anchor-to-file binding is sticky in document order.** The docs anchor a file once and
//!   then list members against it — `**File:** [tag_api.rs](...)` followed by
//!   `` `try_register_tag` (47) / `register_tag` (65) ``. An anchor therefore binds to the most
//!   recent *file-shaped* path mention, which may be on an earlier line. The binding resets at
//!   every markdown heading, because an anchor never spans a section boundary; an anchor with no
//!   bound file (none seen since the last heading) is skipped rather than guessed at. Only
//!   resolvable path mentions rebind, so prose naming a file that does not exist — "NOT the
//!   planned `identifiers/tag_id.rs`" — cannot hijack the anchor of the refs that follow it. A
//!   file that *does* exist but is not the subject, though, hijacks it exactly as it should: the
//!   member list under §2.3 bound to `archive/PHASE-XI-RESULTS.md` because a "See …" link sat two
//!   lines above it, and the EcsMaster member list bound to `FEATURE_MAP.md`. Both were reported
//!   as past-end-of-file the moment the `(N)` form was read, and both were repaired by naming the
//!   subject file on the list's own **API** line rather than by weakening the binding rule.
//! * **Ranges** `:82-104` are shape-checked at their first line only — the end of a range is a
//!   soft claim about extent, not an anchor a reader jumps to — but it must still be coherent
//!   (`end >= start`) and inside the file.
//! * **Several anchors on one line** are all checked, each against the nearest path mention to
//!   its *left* — FEATURE_MAP.md's storage-kind row runs
//!   `[component_registry/mod.rs](...):323 … (:373) … (:433) … ([tags.rs](...):134)`, and the
//!   leading four are checked against `mod.rs` while the last is checked against `tags.rs`.
//!   A `(:N)` is the colon form inside brackets, not the bare form: the `(` is followed by `:`,
//!   not by a digit, so exactly one anchor is produced and nothing is counted twice.
//! * **Citations against a path that does not exist are skipped**, because the path check
//!   already reports that file once. Otherwise one dead path would emit a fresh anchor failure
//!   for every member listed under it and bury the real finding.
//! * **A deliberately non-definition anchor is marked, not exempted wholesale.** A trailing `~`
//!   — `:N~` or `(N~)` — waives the *definition* checks for that one anchor; the file must still
//!   exist and line N must still be within it. This keeps the check at full strength for every
//!   other anchor and leaves each waiver greppable instead of forcing the shape test to be
//!   loosened for everyone. 29 anchors carry it today, on 19 lines — struct fields
//!   (`Archetype::enable_store`, `ObserverLists::by_kind_component`,
//!   `ArchetypeMaster::observer_registry`), enforcement sites inside a derive body, the four call
//!   sites of the enable-store 0%-gate, the `dispatch.rs` OBS-FIRE-LOOP module-doc invariant, and
//!   two physics sites that cite a BEHAVIOUR rather than a declaration — the once-per-step read of
//!   `PhysicsConfig::sdf_narrowphase` (`systems.rs:557~`) and the `Manual` early return that makes
//!   the broadphase policy opt-in (`broadphase_policy.rs:186~`). Both were red under the shape
//!   test for months, and the repair a renumber would have made — moving them onto the enclosing
//!   `fn` — would have destroyed exactly the claim the prose makes.
//!   Enumerate them from `docs/` with
//!   `grep -oE '[:(][0-9]+(-[0-9]+)?~' FEATURE_MAP.md SYSTEMS.md ARCHITECTURE.md | wc -l`. Both
//!   loosenings in that pattern are load-bearing and each was measured: dropping the `[:(]`
//!   alternative to a bare `:` reports 28, because one waiver is written in the parenthesised
//!   form (`(65~`, FEATURE_MAP.md:796); dropping `(-[0-9]+)?` also reports 28, because one is
//!   written on a range (`dispatch.rs:19-33~`, SYSTEMS.md:445). Use `-o`, not `-n`: several of
//!   these lines carry more than one waiver, so counting lines reports 19, not 29.
//!   ⚠️ **These four numbers have now gone stale TWICE, and the second time nobody noticed.**
//!   They read 26/16/25/25, and the commit that un-waived one anchor in SYSTEMS.md re-derived
//!   them to 25/15/24/24 — a re-derivation that was ALREADY WRONG IN THE OTHER DIRECTION:
//!   piping `git show 4a363678:docs/{FEATURE_MAP,SYSTEMS,ARCHITECTURE}.md` through the pattern
//!   above measures **27/17/26/26 on the very tree that shipped the sentence**, so the corpus
//!   had gained two waivers the sentence never saw. This is the failure mode of a measured
//!   INPUT: it rots with no edit to the sentence stating it, so `git log -S` on the prose finds
//!   nothing and only re-running the grep does. 29/19/28/28 is re-derived from the current tree,
//!   not obtained by adding the two waivers this repair introduced.
//!   ⚠️ **The two exemplar citations in this paragraph are anchors into the gated documents, and
//!   this gate structurally cannot check them** — it scans `docs/`, never `tests/`. Both had
//!   rotted: `(65~` moved 756 → 796 and `dispatch.rs:19-33~` moved 435 → 445, silently, under a
//!   green gate. A citation is live only where a checker reads it.
//!   This pattern deliberately covers only the three navigation documents. ⚠️ Its closing claim,
//!   that the waiver-first spelling `:N~-M` has "zero occurrences in these three", was FALSE when
//!   written: SYSTEMS.md:1317 carries `system_meta.rs:172~-188`, and `git show 4a363678` finds it
//!   on the same line there too. The counts are unaffected (the pattern matches its `:172~` head
//!   either way), but rev 12's range repair has a live subject in the navigation docs as well,
//!   not only in the meshlet plan.
//! * **A continuation inherits a SOURCE file across lines and a DOCUMENT only on its own line.**
//!   The sticky binding models a member table — one `**File:** foo.rs` header, a list of members
//!   under it — and refusing that would delete the densest citations in the corpus. Measured over
//!   the nine documents: **373 continuations inherit a target named on an earlier line, and 370 of
//!   them inherit a source file.** The remaining 3 inherited a *document*, all three in one plan,
//!   and all three were wrong: one named a sibling that does not contain the sketch it cites, one
//!   named the right document and the wrong lines, one named its own document and a range holding
//!   an unrelated subject. So the document case is REFUSED and pinned at zero, and the anchors are
//!   rewritten to name their document beside the number. A refusal rather than a ledger, because a
//!   ledger counts anchors and does not move when a counted anchor's NUMBER changes — which is the
//!   same defect as the line-counting marker ceiling below.
//! * **Historical quotes** — a line that deliberately reproduces a former, now-wrong anchor
//!   ("this used to say ...") carries `<!-- doc-anchor-ignore -->` and is skipped whole. This is
//!   an explicit opt-out rather than a heuristic on words like "formerly": a heuristic would
//!   silently switch the gate off on ordinary lines that happen to use the word.
//!   ⚠️ **The marker's ledger counts what it SILENCES, not how many lines carry it**, and the
//!   difference was measured: appending a real anchor violation *and* a real path violation to an
//!   already-marked line left the suite at exit 0 with the line ceiling unmoved, while the same
//!   citation one line away reds. The line is therefore re-scanned with the marker removed and its
//!   sticky binding restored beside it, and the violation count is pinned per document — and a
//!   marker that silences NOTHING fails, exactly as a `<!-- doc-path-planned -->` over a file that
//!   exists does. Three were dead when that half was armed: two over `Lands.` paths that are on
//!   disk under the names they write, and one over a line carrying no citation at all.
//!   ⚠️ This bullet used to end "No line in the three documents needs it today", and that was
//!   already false when written — the meshlet plan carries two, at its `vg_corpus_ingest.rs` and
//!   `vg_density_census.rs` mentions. The sentence survived because nothing re-derived it; it is a
//!   count, so it is not restated here either. `grep -c doc-anchor-ignore docs/*.md` answers it.
//! * **A citation's own file fragment overrides the sticky binding.** Where an anchor is written
//!   directly after a path fragment — `` `component_registry/tags.rs:134` `` — that fragment is the
//!   document's statement of which file it means, and it wins over whatever path the section named
//!   earlier. It is resolved by [`resolve_unique_fragment`], which requires the fragment to match
//!   **exactly one** file in the tree; zero or several matches mean the anchor is SKIPPED, never
//!   guessed at, and the skip is counted and pinned by
//!   [`unbindable_fragments_are_reported_and_pinned`]. A resolved fragment also rebinds the sticky
//!   target, so the `` `:155` `` continuations that follow it inherit the right file.
//!   This rule is what makes the reflection plans checkable at all: they write **one row per
//!   claim**, each row naming its own file, which is the exact inverse of the member tables the
//!   sticky binding was designed for. Measured when they entered scope — of their 634 citations,
//!   only 53 were rooted `crates/...`, against 303 bare fragments and 258 bare `:N` continuations.
//! * **A plan may name a file it has not built.** `<!-- doc-path-planned -->` waives the existence
//!   check for one line's mentions and nothing else; see [`PLANNED_MARKER`]. A marker that waives
//!   NOTHING -- every path on its line already on disk -- is itself a violation, because the
//!   waiver outlives the deliverable and silences the next path written on that line; see
//!   [`DocScan::stale_planned`].
//! * **A glob is not a path, and a ratio is not an anchor.** `docs/PHASE-*-RESULTS.md` names a
//!   family, not a file, and `(3840/1920 = 2.000)` is arithmetic. Both were being read as claims;
//!   both rejections are at the point of extraction, with the measurement, in `scan_line`.
//!
//! # Scope
//!
//! The three navigation documents, `MESHLET-VIRTUAL-GEOMETRY-PLAN.md`, and the reflection
//! campaign's five planning and analysis documents. The rest of `docs/` is audit and results files:
//! dated records of what was believed at a point in time. Rewriting their anchors to match today's
//! source would falsify the record, so they stay out of scope.
//!
//! ⚠️ **The reflection documents were added because their absence had been MEASURED, twice.** An
//! implementer's own edit shifted the lines its plan cited and nothing reddened; the rot was found
//! by a human reading pass, and a follow-up then enumerated ten more anchors that had been stale
//! before that. Arming the gate over them — before repairing anything, which is the only order that
//! proves the gate can see them — reported **12 dead paths and 150 stale anchors of 231 checked**.
//! Classifying that list is what found the binding defect above: the majority were not stale, they
//! were being checked against a file the document never named. Repairing the binding took the same
//! corpus to **541 anchors checked**, and the reds that survived were the real ones.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The internal documents `CLAUDE.md` points agents at for navigation, plus the campaign plans
/// that cite the source tree densely enough to rot between rungs: the three navigation documents,
/// the virtual-geometry plan, and the reflection campaign's five.
///
/// The plan was ADDED, REMOVED, and ADDED AGAIN, and the round trip is worth recording because it
/// measured a real limit of this gate rather than a preference.
///
/// It was the strongest candidate in the tree: its §12 appendix used to open *"Every line below was
/// opened or grepped while writing this revision"*, and that blanket claim was **false in four
/// consecutive revisions**. Exactly the promise a machine should keep. The first attempt failed
/// because the plan cited bare basenames in prose (`` `mesh_assets.rs:252` ``) while this scanner
/// binds an anchor to the nearest resolvable path mention — measured then: 83 "stale" of 146,
/// dominated by misbindings.
///
/// Converting the citations to link form was the named follow-up and it landed, which raised the
/// bound set to 201 and let the plan in. **Read the printed decomposition before trusting a green
/// run here:** roughly half the plan's anchors carry the `~` waiver, because this gate models an
/// anchor as pointing at a DEFINITION while the plan cites EVIDENCE lines — a usage flag, an enum
/// variant, a comment asserting the fact being cited. Re-pointing those at definitions would move
/// the citations away from the evidence they cite. A waived anchor asserts only that the line
/// number exists in the file: `check_anchor` returns at the waiver branch before the shape test
/// runs, so a waived anchor that is simply WRONG about which line holds the symbol still passes.
/// What the plan's membership does buy is the class that actually rots — a cited file that
/// disappears or shrinks — and it caught three dead paths on the first run.
///
/// # The register corpus (added 2026-09-10)
///
/// The four documents above are the *navigation* docs. The six below are the **register** corpus —
/// `OPEN-QUESTIONS.md`, the four `gaia/` documents and the revision note that binds them — and they
/// were let in because the class of defect this gate exists to catch was found there by hand, not
/// by any check: a `light.rs:1207` citation dropped in a merge resolution of `gaia/DECISIONS.md`,
/// with nothing in the repository able to report it. These documents cite source lines as densely
/// as `SYSTEMS.md` does and rot the same way, and their citations are the evidence a ruled ballot
/// stands on, so a dead one costs more than a dead navigation anchor: it makes a *decision*
/// unverifiable.
///
/// ⚠️ **They live one directory down, and that alone broke `resolve`** — see its own comment. The
/// widening therefore is not "add six strings"; the path rule had to become a function of the
/// citing document.
///
/// The reflection campaign's five planning documents, added 2026-08-21.
///
/// They were added because the absence was MEASURED, twice in two rungs: an implementer's own edit
/// shifted the lines its plan cited, nothing reddened, and the rot was found by a reading pass. A
/// follow-up then enumerated ten more anchors that had already been stale before those rungs.
///
/// ⚠️ **Read the coverage note in the module doc before trusting a green run here.** These five
/// documents write most of their citations as a *bare relative fragment* — `` `format.rs:210-258`
/// ``, `` `component_registry/mod.rs:61` `` — under a section that names the full path once. This
/// scanner binds an anchor to the nearest resolvable path mention, and a bare fragment is not one,
/// so those anchors bind to the section's last full `crates/...` path. That is the intended sticky
/// binding when the fragment names the section's own subject, and a MISBINDING when it names a
/// sibling file. The arming run is documented in the same note.
const GATED_DOCS: &[&str] = &[
    "FEATURE_MAP.md",
    "SYSTEMS.md",
    "ARCHITECTURE.md",
    "MESHLET-VIRTUAL-GEOMETRY-PLAN.md",
    "OPEN-QUESTIONS.md",
    "AETHER-GAIA-REVISION-2026-08-29.md",
    "gaia/CAMPAIGN.md",
    "gaia/DECISIONS.md",
    "gaia/LANGUAGE.md",
    "gaia/PENDING-SYNTAX-PLAN.md",
    "REFLECTION-ANALYSIS.md",
    "REFLECTION-PLAN-BOUNDARY.md",
    "REFLECTION-PLAN-CORE.md",
    "REFLECTION-PLAN-ECS.md",
    "REFLECTION-PLAN-GATES.md",
];

/// The documents whose anchors cite EVIDENCE rather than declarations.
///
/// # Why a genre flag and not 50 waivers
///
/// A register entry cites the line that *states the fact it stands on* — a doc comment reading
/// `/// LINEAR `rgb` color.`, the `scope.spawn_batch(` call site a blast-radius table enumerates,
/// the `assert!(` that proves an alignment. None of those is a definition, and none should be: a
/// renumber onto the enclosing `fn` would move the citation off the evidence it cites, which is
/// the repair this gate's own module doc warns destroys the claim. Measured on the first correctly
/// bound run of the widened corpus: **50 anchors sat on non-definition lines, and inspection put
/// 47 of them in that class** — the other three are content rot and are reported below.
///
/// The meshlet plan already conceded the same genre, with **89 `~` waivers on 176 anchors**, i.e.
/// by hand, one site at a time. GB-8 (`OPEN-QUESTIONS.md`) ruled that out: *no per-site waivers, at
/// any of the four censuses, ever; where a property is not decidable as written, narrow the
/// predicate and print the narrowing in the failure message.* This constant is that narrowing —
/// one declaration per document, greppable, and counted in the printed decomposition so the cost
/// is visible rather than absorbed.
///
/// ⚠️ **It is a real loss of strength and is named as one.** On these documents the shape test is
/// not run. What still holds, and what caught something today: the cited path must exist, the line
/// must be inside the file, a range must be coherent, and — the part deliberately KEPT — where the
/// line's symbols pair one-to-one with its anchors and the cited file declares that symbol, the
/// line must contain it. That last clause is what reports `worker.rs:370` as not defining
/// `push_task`, which is at `:686`.
const EVIDENCE_DOCS: &[&str] = &[
    // ⚠️ The meshlet plan is here for the same reason and it cost ONE anchor to admit, measured:
    // of its 176, exactly 1 sits on a non-definition line without a `~`, and the other 88
    // unwaived ones still pass the shape test unchanged. It was already an evidence document by
    // its own admission — 87 of those 176 anchors carry a hand-applied `~` — so the genre flag
    // states once what the waivers were saying one site at a time. The single anchor is
    // `loaders/obj.rs:39`, the doc-comment line reading "(F-obj — no hashing: see
    // `dedup_corners`)", which is the evidence for the sentence citing it; it was invisible until
    // the fragment binding above started pointing that citation at the file it names.
    "MESHLET-VIRTUAL-GEOMETRY-PLAN.md",
    "OPEN-QUESTIONS.md",
    "AETHER-GAIA-REVISION-2026-08-29.md",
    "gaia/CAMPAIGN.md",
    "gaia/DECISIONS.md",
    "gaia/LANGUAGE.md",
    "gaia/PENDING-SYNTAX-PLAN.md",
];

/// Opt-out marker for a line that quotes a stale anchor on purpose.
const IGNORE_MARKER: &str = "<!-- doc-anchor-ignore -->";

/// Opt-out marker for a line that names an artifact the plan has **not built yet**.
///
/// The navigation documents describe a tree that exists, so every path they name must be on disk.
/// A *plan* also names the files it is going to create — `` **Lands.**
/// `crates/reflect_fixture/tests/boundary_roundtrip.rs` `` — and that is a commitment, not a claim
/// about today's disk. Ten such declarations reddened the path check when the reflection plans
/// entered `GATED_DOCS`, and every available way to silence them was worse than a marker: deleting
/// the path deletes the deliverable's name, `<!-- doc-anchor-ignore -->` says "historical quote"
/// which is false and also drops the line's anchors, and dropping the plans from the path check
/// entirely would forfeit the class that actually rots.
///
/// It waives **only** the existence check, and only on its own line; anchors are unaffected. The
/// count is pinned per document by [`planned_paths_are_reported_and_pinned`], so the marker is a
/// visible ledger of what each plan still owes rather than a way to make the check quiet. When the
/// artifact lands, the marker comes off and the path is checked like any other.
///
/// ⚠️ **"Comes off" is now CHECKED, and it was not before.** The count above moves the moment
/// the file appears, whether or not anyone deleted the marker, so a plan could land a deliverable,
/// decrement its pin, keep the marker, and stay green -- MEASURED. [`DocScan::stale_planned`]
/// carries the other half: a marker on a line whose every path exists is reported and failed by
/// the same test.
const PLANNED_MARKER: &str = "<!-- doc-path-planned -->";

/// The workspace root. This test lives in the root package, so the manifest dir *is* the root.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn docs_dir() -> PathBuf {
    repo_root().join("docs")
}

/// A repo-relative path mentioned by a document.
struct Mention {
    /// Byte column the mention starts at, used to order it against the anchors on the same line.
    col: usize,
    /// As written in the document, for the failure message.
    raw: String,
    /// Resolved against the repository root.
    resolved: PathBuf,
}

/// A line anchor, in either form the documents write: the suffix `:N` after a file name, or the
/// bare parenthesised `(N)` a member table writes under a sticky **File:** header. Both take a
/// `-M` range tail and a `~` waiver, so `:N-M~` and `(N-M~)` are equally legal.
struct Anchor {
    col: usize,
    line_no: usize,
    /// `M` of an `N-M` range. Checked for coherence (`M >= N`) and for being inside the file;
    /// the definition-shape test still applies to `N` alone.
    ///
    /// ⚠️ **Stated as a limit of the instrument, not a bug in it: `M` is bounds-checked, never
    /// content-checked.** A range may therefore end in the middle of an unrelated construct — or
    /// short of the very line that carries the fact the citation is making — and this census stays
    /// green, because it has verified only that `M` is a line the file has. MEASURED:
    /// `REFLECTION-PLAN-CORE.md` cited `boyko_reflect/src/registry.rs:87-118` for *"the table is
    /// write-once, first writer wins"*; `:87` is the right signature, but `:118` is the middle of an
    /// `assert!` condition and the `OnceLock::set` that IS the write-once claim sits at `:127`,
    /// outside the cited range. The repaired citation is `:87-128`. A reader following a
    /// census-green range can still land on the wrong text; only the anchor line is shape-checked.
    range_end: Option<usize>,
    /// `true` when written with a trailing `~` — the definition checks are waived for this
    /// anchor only.
    shape_waived: bool,
}

/// Whether the `(` at `at` immediately follows a closing inline-code span, allowing spaces
/// between — `` `spawn_one::<A>` (582) ``. That is the shape of every bare-`(N)` citation in these
/// documents, and it is what separates one from an enumeration marker in prose.
fn labels_a_code_span(bytes: &[u8], at: usize) -> bool {
    let mut j = at;
    while j > 0 && bytes[j - 1] == b' ' {
        j -= 1;
    }
    j > 0 && bytes[j - 1] == b'`'
}

/// Repo-root-relative prefixes a **bare** (unlinked) path mention may start with.
///
/// This was `crates/` alone, and the omission was measured when the reflection documents entered
/// `GATED_DOCS`: they cite `.github/workflows/ci.yml:62` and `docs/FEATURE_MAP.md:112` in prose,
/// neither of which the scan could see. An unseen mention is not a skipped check — it is a
/// **misbinding**, because the anchor after it falls through to the last `crates/...` path instead,
/// and the run then reports that file's line count. `REFLECTION-PLAN-GATES.md:1684` cited seven
/// `ci.yml` legs and every one was checked against `crates/profile_fixture/Cargo.toml` (18 lines).
///
/// ~~⚠️ **That `:1684` is prose, and nothing checks it.** It was written `:1641`, was already off
/// by two when written, and drifted to `:1684` when a discharge banner went in above it — found by
/// reading, not by a run. The reason is structural and worth naming rather than repairing twice:
/// **this census scans `.md` files for citations into `.rs`, never the reverse.** A `.rs` file
/// citing a `.md` line — this comment, and any other — is outside every census in the tree, so its
/// line numbers rot silently. Recorded as a known gap, not scheduled: the reverse direction is a
/// second scanner with its own false-binding surface, and the citations it would cover are
/// explanatory rather than load-bearing.~~
///
/// **SCHEDULED AND LANDED**, by
/// [`md_line_citations_written_inside_rust_sources_are_bounds_checked`]. The struck paragraph is
/// kept because its last clause is the part that failed: *"explanatory rather than load-bearing"*
/// was the reason not to build it, and while it stood, this file accumulated two more stale
/// citations of its own — `:187` and `:959`, both repaired at that landing, both found by reading.
/// A citation that no gate reads rots at the rate of the document it points into, whatever it is
/// explaining. `:1684` itself is now checked, and today it holds.
///
/// ⚠️ **And "the reverse direction" was not one direction.** That landing scanned `.rs` → `.md` and
/// filtered its targets to `.md` by construction, so `.rs` → `.rs` stayed unread by either census
/// while this comment declared the reverse covered. Built by
/// [`rs_line_citations_written_inside_rust_sources_are_bounds_checked`], it found **12 citations
/// past the end of the file they name** on its first run, one of them in a file the landing that
/// wrote this paragraph had itself modified. A gap named in the singular is a gap measured in the
/// singular.
///
/// ⚠️ **`src/` and `tests/` are deliberately NOT here, and adding them would be a regression.**
/// Both exist at the repository root *and* inside every crate, so a bare `src/lib.rs` or
/// `tests/foo.rs` is ambiguous between root-relative and crate-relative — and these documents write
/// far more of the second kind (measured over the five: 22 bare `src/…`, 47 bare `tests/…`).
/// Treating them as root-relative would resolve `src/lib.rs` onto the workspace root's own file,
/// which EXISTS, so the path check would pass while every anchor behind it bound to the wrong crate
/// — a silent wrong answer, strictly worse than the current silence. Those citations are reached as
/// anchors instead, by [`resolve_unique_fragment`], which refuses rather than guesses.
///
/// The consequence, stated plainly so it is not mistaken for coverage: a bare `tests/…`, `src/…`,
/// `book/…`, `scripts/…` or `tools/…` mention is **not existence-checked**. Only its anchor is, and
/// only when the fragment names one file. Write it as a markdown link or a full `crates/…` path to
/// bring it under the path check.
const ROOT_PREFIXES: &[&str] = &["crates/", "docs/", ".github/"];

/// Characters that may appear inside a bare `crates/...` mention.
fn is_path_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'/' | b'-')
}

/// Trailing sentence punctuation is not part of the path.
fn trim_path_punctuation(s: &str) -> &str {
    s.trim_end_matches(['.', ',', ';', '-'])
}

/// Resolve a mention to an absolute path.
///
/// Markdown link targets are relative to **the document that wrote them**, which is `docs/` only
/// for a document that sits directly in `docs/`. ⚠️ This used to be hard-coded to `docs_dir()`,
/// with a special case peeling exactly one `../` off and re-rooting at the repository — a shape
/// that is correct for `docs/FEATURE_MAP.md` and silently WRONG one directory down: measured on
/// `docs/gaia/DECISIONS.md`, whose links are written `../../crates/…`, the old rule produced
/// `<repo>/../crates/…` and reported every one of them as a dead path. Resolving against the
/// citing document's own directory covers both, because `docs/` *is* that directory for the four
/// documents the gate started with. Bare `crates/...` mentions in prose keep their own rule: they
/// are written from the repository root regardless of where the document lives.
fn resolve(raw: &str, doc_dir: &Path) -> Option<PathBuf> {
    if raw.starts_with("http://") || raw.starts_with("https://") || raw.starts_with("mailto:") {
        return None;
    }
    // Strip a `#fragment` — the anchor within a document is not part of the file path.
    let base = raw.split('#').next().unwrap_or("");
    if base.is_empty() {
        return None;
    }
    // `ROOT_PREFIXES` (the lane's widening) decides which bare prefixes are repo-root-relative;
    // everything else is relative to the CITING document's directory (the line's widening, needed
    // because a `GATED_DOCS` entry may name a subdirectory). A `../` head needs no arm of its own:
    // `doc_dir.join` produces it verbatim and the unconditional `normalize` below collapses it.
    let path = if ROOT_PREFIXES.iter().any(|p| base.starts_with(p)) {
        repo_root().join(base)
    } else {
        doc_dir.join(base)
    };
    // `..` segments must not defeat the existence check; normalising unconditionally costs one
    // component walk and removes the "how many `../` did we anticipate" question entirely.
    Some(normalize(&path))
}

/// Collapse `a/b/../c` to `a/c` so `Path::exists` sees a real path on every platform.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Extract every path mention and every line anchor from one line of a document.
fn scan_line(text: &str, doc_dir: &Path) -> (Vec<Mention>, Vec<Anchor>) {
    let bytes = text.as_bytes();
    let mut mentions: Vec<Mention> = Vec::new();
    // Byte spans already consumed by a markdown link target, so the bare-mention scan below does
    // not report the same path a second time.
    let mut link_spans: Vec<(usize, usize)> = Vec::new();

    // 1. Markdown link targets: `](target)` or `](target "title")`.
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b']' && bytes[i + 1] == b'(' {
            // The `[` that opens this link's label, so an anchor written inside the label binds to
            // the link's own target. `[` is ASCII, so a reverse byte scan cannot land inside a
            // multi-byte character.
            let label_open = bytes[..i].iter().rposition(|&b| b == b'[');
            let start = i + 2;
            if let Some(rel_end) = bytes[start..].iter().position(|&b| b == b')') {
                let end = start + rel_end;
                if let Some(inner) = text.get(start..end) {
                    // A link title after the target is not part of the path.
                    let target = inner.split_whitespace().next().unwrap_or("");
                    link_spans.push((start, end));
                    if !target.is_empty()
                        && !target.starts_with('#')
                        && let Some(resolved) = resolve(target, doc_dir)
                    {
                        // ⚠️ The mention is recorded at the `[` that opens the link's LABEL, not
                        // at the target text, because anchors bind to the nearest mention on
                        // their LEFT and the register corpus writes the citation INSIDE the
                        // label: `` [`bind_system.rs:56-60`](../crates/…/bind_system.rs) ``,
                        // where the navigation docs write it after the link,
                        // `[bind_system.rs](…):56`. Recorded at the target, the label-form anchor
                        // sits to the left of its own link and binds to whatever file was cited
                        // previously — measured on OPEN-QUESTIONS.md:82, where `:56-60` bound to
                        // a `light.rs` link from an earlier table row and was reported stale
                        // against a file it never named. Moving the column cannot mis-order the
                        // mentions on a line, because a label always precedes its own target.
                        let col = label_open.unwrap_or(start);
                        mentions.push(Mention {
                            col,
                            raw: target.to_string(),
                            resolved,
                        });
                    }
                }
                i = end + 1;
                continue;
            }
        }
        i += 1;
    }

    // 2. Bare root-relative mentions in prose or backticks — see `ROOT_PREFIXES`.
    //
    // ⚠️ `docs/` was added with the register corpus, BEFORE `ROOT_PREFIXES` generalised the list.
    // Those documents cite each other by repo-relative path with a line number —
    // `` `docs/OPEN-QUESTIONS.md:250` `` — and with only `crates/` scanned, no mention was
    // produced, so the `:250` bound to whatever SOURCE file was cited last and was reported stale
    // against it. Measured on OPEN-QUESTIONS.md:124, where `docs/OPEN-QUESTIONS.md:250`, `:282`
    // and `:198` were all charged to `light.rs`. The prefix set is now data, not a literal pair.
    let mut i = 0;
    'outer: while i < bytes.len() {
        for needle in ROOT_PREFIXES {
            let n = needle.as_bytes();
            if i + n.len() <= bytes.len()
                && &bytes[i..i + n.len()] == n
                && !link_spans.iter().any(|&(s, e)| i >= s && i < e)
                // Only a path *start*: `subcrates/` and the tail of `../crates/` handled elsewhere.
                && (i == 0 || !is_path_byte(bytes[i - 1]))
            {
                let mut end = i;
                while end < bytes.len() && is_path_byte(bytes[end]) {
                    end += 1;
                }
                // A GLOB is not a claim that one file exists. `*` and `?` are not path bytes, so
                // the scan stops in front of them and the truncated head would be checked as if it
                // were a whole path: `docs/PHASE-*-RESULTS.md` became `docs/PHASE-`, then
                // `docs/PHASE` after punctuation trimming, and was reported dead in FEATURE_MAP.md
                // and SYSTEMS.md the moment `docs/` joined ROOT_PREFIXES. The pattern is prose
                // about a family of files; only a literal path asserts that one of them is there.
                let is_glob = matches!(bytes.get(end).copied(), Some(b'*' | b'?'));
                if let Some(slice) = text.get(i..end)
                    && !is_glob
                {
                    let raw = trim_path_punctuation(slice);
                    if !raw.is_empty()
                        && let Some(resolved) = resolve(raw, doc_dir)
                    {
                        mentions.push(Mention {
                            col: i,
                            raw: raw.to_string(),
                            resolved,
                        });
                    }
                }
                i = end;
                continue 'outer;
            }
        }
        i += 1;
    }

    // 3. Line anchors, in both forms these documents write: the suffix form `file.rs:N` and the
    //    bare parenthesised `(N)` the member tables use under a sticky `**File:**` header. Both
    //    accept a `-M` range tail and the `~` waiver.
    let mut anchors: Vec<Anchor> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let paren = bytes[i] == b'(';
        let opens = match bytes[i] {
            // A digit before the colon means a clock time or a ratio, not an anchor.
            b':' => !(i > 0 && bytes[i - 1].is_ascii_digit()),
            // A `(` that opens an argument list is a quantity, not a citation — `O(1)`,
            // `wrapping_add(1)` and `pool_reserve_rows(0)` all occur in these documents.
            // Requiring that the byte in front of it is not an identifier byte separates the two.
            //
            // ⚠️ **And it must LABEL a code span.** The bare `(N)` form exists because a member
            // table writes one **File:** header and then a line number per member —
            // `` `spawn_one::<A>` (582) `` — so the citation always sits immediately after the
            // backticked symbol it labels. Prose does not: the register corpus enumerates with
            // `**Three parts.** (1) … (2) … (3)`, and with only the two structural rules above
            // every one of those markers was read as a citation and charged to whichever file the
            // section had last named. Measured on the first widened run: OPEN-QUESTIONS.md alone
            // reported 87 stale anchors of 140, and the enumeration markers were the largest
            // single class. Requiring a closing backtick before the `(` is a NARROWING of the
            // predicate, not a per-site waiver (GB-8), and it is measured not to move the four
            // navigation documents' anchor counts.
            b'(' => {
                !(i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_'))
                    && labels_a_code_span(bytes, i)
            }
            _ => false,
        };
        // A markdown link target's own `(` sits one byte in front of the recorded span.
        let in_link = link_spans
            .iter()
            .any(|&(s, e)| (i >= s && i < e) || (paren && i + 1 == s));
        if opens && !in_link {
            let mut end = i + 1;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end > i + 1 {
                let line_no: usize = text[i + 1..end].parse().unwrap_or(0);
                // A range `:N-M` is shape-checked at N only; the tail is captured (not merely
                // skipped) so an end that precedes its start cannot hide behind the first line.
                //
                // ⚠️ The waiver may sit on EITHER side of the range, and until Rev 12 this scanner
                // only recognised one side. It tested `-` before `~`, so it parsed `:N-M~` and
                // stopped dead at the `~` in `:N~-M`. Measured on the meshlet plan: **34
                // occurrences waiver-first, 23 plain `:N-M`** — `range_end` was `None` for the 34,
                // so the end-of-range check below never ran on them. ⚠️ Rev 12 first wrote that
                // this was "every one of its ranges" and that the check "never ran on that
                // document once"; both are FALSE — the 23 plain citations parsed a tail and
                // reached both assertions before this repair existed. The 34 are reason enough,
                // and the false half is recorded rather than quietly dropped because it was a
                // volunteered claim in a repair, the exact class this campaign measures.
                let mut after = end;
                let mut shape_waived = false;
                if after < bytes.len() && bytes[after] == b'~' {
                    shape_waived = true;
                    after += 1;
                }
                let mut range_end = None;
                if after < bytes.len() && bytes[after] == b'-' {
                    let mut t = after + 1;
                    while t < bytes.len() && bytes[t].is_ascii_digit() {
                        t += 1;
                    }
                    if t > after + 1 {
                        range_end = text[after + 1..t].parse::<usize>().ok();
                        after = t;
                    }
                }
                if !shape_waived && after < bytes.len() && bytes[after] == b'~' {
                    shape_waived = true;
                    after += 1;
                }
                // The parenthesised form must then close, or go on to the next member of a
                // `(a) / (b)` list, or introduce a parenthetical note. A quantity that merely
                // begins with digits never does: `(512 B = …)`, `(4096 with …)`, `(1024-bit
                // dedup)`, `(0%-gate …)`, `(19 members)`, `(12.6)`, `(14a)` are all rejected here
                // rather than by a unit blacklist.
                // `after` already steps past whichever side the waiver was written on.
                // ⚠️ The `/` alternative exists for a `(a) / (b)` member list, where the slash is a
                // SEPARATOR. A slash followed directly by another digit is a RATIO, and reading it
                // as a citation was a latent false positive: the meshlet plan's
                // `(3840/1920 = 2.000)` was parsed as an anchor on line 3840. It stayed invisible
                // only because the file it happened to bind to was large enough; the moment
                // `docs/` joined `ROOT_PREFIXES` it rebound to a 94-line document and reported
                // ``past end of file``. A real list writes `(a) / (b)` with spaces.
                // ⚠️ A bare `(N)` must be a MEMBER CITATION, which in these documents always
                // trails the backticked symbol it cites — `` `spawn_one::<A>` (582) ``. A `(1)`
                // that opens an enumerated clause — "…recorded at execution.** (1) Gate 5 requires
                // …" — satisfies every other rule here and was read as an anchor on line 1. That is
                // not hypothetical: it reported four such "stale" anchors in
                // REFLECTION-PLAN-GATES.md, and the first repair pass WAIVED them, writing `(1~)`
                // `(2~)` `(3~)` into the prose — a scanner false positive laundered into the
                // document as if it were a deliberate citation. Requiring a closing backtick or
                // `)` in front separates the two structurally; measured over the four incumbent
                // documents, every genuine bare citation has one and no enumerator does.
                let mut back = i;
                while back > 0 && bytes[back - 1] == b' ' {
                    back -= 1;
                }
                let cited_symbol_in_front =
                    back > 0 && matches!(bytes[back - 1], b'`' | b')');
                let after_byte = bytes.get(after).copied();
                let ratio = after_byte == Some(b'/')
                    && bytes.get(after + 1).is_some_and(u8::is_ascii_digit);
                let closes = !paren
                    || (!ratio
                        && cited_symbol_in_front
                        && matches!(after_byte, Some(b')' | b',' | b'/')));
                if line_no > 0 && closes {
                    anchors.push(Anchor {
                        col: i,
                        line_no,
                        range_end,
                        shape_waived,
                    });
                }
                i = after;
                continue;
            }
        }
        i += 1;
    }

    mentions.sort_by_key(|m| m.col);
    anchors.sort_by_key(|a| a.col);
    (mentions, anchors)
}

/// A path that names a file (has an extension) can anchor the `:N` and `(N)` refs that follow
/// it; a directory link cannot.
fn is_file_shaped(raw: &str) -> bool {
    let base = raw.split('#').next().unwrap_or("");
    !base.ends_with('/') && Path::new(base).extension().is_some()
}

/// Does line `n` of a source file look like a definition rather than a body line?
fn looks_like_definition(line: &str, ext: &str) -> bool {
    let t = line.trim();
    if t.is_empty() {
        return false;
    }
    if ext.eq_ignore_ascii_case("toml") {
        return t.starts_with('[');
    }
    if !ext.eq_ignore_ascii_case("rs") {
        // No definition grammar known for this file type; existence of the line is all we claim.
        return true;
    }
    // A comment or an attribute is adjacent to a definition, not one.
    if t.starts_with("//") || t.starts_with("/*") || t.starts_with('*') || t.starts_with("#[") {
        return false;
    }
    // ⚠️ These were written WITH a trailing space, and the space silently excluded every
    // definition whose keyword is followed by a generic parameter list. Measured when the
    // reflection documents entered `GATED_DOCS`: `impl<S: States> Resource for State<S> {` —
    // `state.rs:43`, cited by three separate paragraphs of REFLECTION-ANALYSIS.md — was reported
    // ``is not a definition``, because `impl<` is not `impl `. The boundary is now checked on BOTH
    // sides instead, so `impl<T>` and `struct Foo<T>` qualify while `implementation` and
    // `type_name` still do not.
    const KEYWORDS: &[&str] = &[
        "fn",
        "struct",
        "enum",
        "const",
        "static",
        "impl",
        "trait",
        "type",
        "macro_rules!",
        "union",
        "mod",
    ];
    let b = t.as_bytes();
    let is_ident_byte = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let word_at = |kw: &str| {
        t.match_indices(kw).any(|(idx, _)| {
            // Word boundary on both sides: the keyword must be neither the tail nor the head of a
            // longer identifier.
            let before_ok = idx == 0 || !is_ident_byte(b[idx - 1]);
            let after = idx + kw.len();
            let after_ok = after >= b.len() || !is_ident_byte(b[after]);
            before_ok && after_ok
        })
    };
    // ⚠️ `pub` keeps its trailing space and is NOT given the two-sided boundary above, because it
    // is a VISIBILITY QUALIFIER rather than an item keyword — every item it can precede is already
    // matched by its own keyword (`pub(crate) fn` by `fn`, `pub struct` by `struct`). What `pub`
    // alone still reaches is a STRUCT FIELD, `pub next: Option<Row>`, which is not a definition and
    // is the canonical `~` waiver class in these documents. Two-siding it would additionally match
    // `pub(crate) enable_store: EnableStore,` — measured: doing so turned three long-standing
    // waivers in FEATURE_MAP.md and SYSTEMS.md into over-waivers, i.e. it reclassified two
    // documents' struct-field citations by widening a qualifier, which is not the defect that was
    // being fixed. The defect was `impl<S: States>`, and it is fixed above.
    //
    // It therefore keeps the ORIGINAL one-sided rule: the space is already the right-hand
    // boundary, and demanding a non-identifier byte after it rejects `pub use scope::Scope;` and
    // `pub geometry_slot: u32,` — measured, three fresh stale anchors in SYSTEMS.md and one in the
    // meshlet plan, on citations that had been correct for their whole life.
    let pub_prefixed = t.match_indices("pub ").any(|(idx, _)| {
        idx == 0 || !is_ident_byte(b[idx - 1])
    });
    pub_prefixed || KEYWORDS.iter().copied().any(word_at)
}

/// Rust keywords, which a doc line uses as prose (`for x in &q`) far more often than as a symbol.
const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "box", "break", "const", "continue", "crate", "dyn", "else", "enum",
    "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "macro", "match", "mod",
    "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait",
    "true", "type", "union", "unsafe", "use", "where", "while",
];

/// Accept `name` as a symbol, or reject it as prose.
fn validate_ident(name: &str) -> Option<String> {
    // Two characters is below the noise floor: `Ok`, `T`, `id` would pair with anything.
    if name.len() < 3
        || !name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        || RUST_KEYWORDS.contains(&name)
    {
        return None;
    }
    Some(name.to_string())
}

/// The last symbol the span *calls*: `commands.entity(e).enable::<T>()` → `enable`.
///
/// The docs name a member by writing a call to it, and the member the anchor cites is the one at
/// the end of the chain — the receiver in front of it is context, not the claim.
fn last_called_ident(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut best = None;
    for i in 0..b.len() {
        let is_call = b[i] == b'('
            || (b[i] == b':' && b.get(i + 1) == Some(&b':') && b.get(i + 2) == Some(&b'<'));
        if !is_call {
            continue;
        }
        let mut start = i;
        while start > 0 && (b[start - 1].is_ascii_alphanumeric() || b[start - 1] == b'_') {
            start -= 1;
        }
        if start < i && let Some(found) = validate_ident(&s[start..i]) {
            best = Some(found);
        }
    }
    best
}

/// Reduce one backticked span to the bare identifier it names, or `None` if it names no single
/// identifier.
///
/// The docs write members three ways: as a declaration (`` `const STORAGE_IS_BITSET = true` ``),
/// as a call (`` `try_register_tag(name) -> Option<TagId>` ``, `` `.add_tag(TagId)` ``), or bare
/// and possibly generic (`` `POOL_MIN_SLAB` ``, `` `Query<D, F>` ``, `` `C::STORAGE_IS_BITSET` ``).
/// Everything else — a path (`` `identifiers/tag_id.rs` ``), a filename (`` `mod.rs` ``), an
/// attribute, a code phrase (`` `page = row >> 12` ``, `` `for x in &q` ``) — is prose about the
/// code rather than a name for it, and yields `None` so it can never be paired with an anchor.
fn core_ident(span: &str) -> Option<String> {
    let s = span.trim();
    if s.contains('/') {
        return None;
    }
    if let Some(declared) = leading_decl_name(s) {
        return Some(declared);
    }
    if let Some(called) = last_called_ident(s) {
        return Some(called);
    }
    let head_len = s
        .bytes()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b':')
        .count();
    let (head, tail) = s.split_at(head_len);
    // A bare name must be the whole span, or be followed only by its generic arguments or its
    // fields (`Column { ptr, stride }`). Anything else after it (`.rs`, ` = row >> 12`,
    // `_{add,insert}`) means the span is not just a name.
    let named_shape = tail.is_empty() || tail.starts_with('<') || tail.trim_start().starts_with('{');
    if head.is_empty() || !named_shape {
        return None;
    }
    validate_ident(head.trim_end_matches(':').rsplit("::").next()?)
}

/// Every identifier the doc text on this line offers, in column order.
fn backticked_idents(text: &str) -> Vec<String> {
    text.split('`')
        .skip(1)
        .step_by(2)
        .filter_map(core_ident)
        .collect()
}

/// The identifier a fenced pseudo-declaration line declares — `pub enum   ObserverKind {` →
/// `ObserverKind`. Fenced blocks carry no backticks, so this is where their identity claim lives.
fn leading_decl_name(line: &str) -> Option<String> {
    let mut t = line.trim();
    // Visibility / safety qualifiers stack in any order before the item keyword. `const` is one
    // of them only in `const fn`; elsewhere it *is* the item keyword.
    loop {
        let before = t;
        for pre in ["pub(crate) ", "pub(super) ", "pub ", "unsafe ", "async ", "extern "] {
            t = t.strip_prefix(pre).unwrap_or(t).trim_start();
        }
        if t.starts_with("const fn ") {
            t = t["const ".len()..].trim_start();
        }
        if t == before {
            break;
        }
    }
    const ITEM_KEYWORDS: &[&str] = &[
        "struct ", "enum ", "trait ", "type ", "const ", "static ", "fn ", "union ", "mod ",
    ];
    let rest = ITEM_KEYWORDS
        .iter()
        .find_map(|kw| t.strip_prefix(kw))?
        .trim_start();
    let name: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if name.len() < 3 || !name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
        return None;
    }
    Some(name)
}

/// Does `ident` occur in `line` as a whole word rather than inside a longer identifier?
fn contains_word(line: &str, ident: &str) -> bool {
    let b = line.as_bytes();
    line.match_indices(ident).any(|(idx, _)| {
        let ok_before = idx == 0 || !(b[idx - 1].is_ascii_alphanumeric() || b[idx - 1] == b'_');
        let after = idx + ident.len();
        let ok_after = after >= b.len() || !(b[after].is_ascii_alphanumeric() || b[after] == b'_');
        ok_before && ok_after
    })
}

// ────────────────────────────────────────────────────────────────────────────
// Doc-to-doc citations: the CONTENT check.
//
// ⚠️ **A `X.md:N` anchor was BOUNDS-CHECKED ONLY, and that is a gate that cannot
// fail on the defect it exists to catch.** `looks_like_definition` returns
// `true` unconditionally for every non-`.rs`, non-`.toml` extension — *"no
// definition grammar known for this file type; existence of the line is all we
// claim"* — so a citation into another document asserted nothing beyond
// `N <= line_count`.
//
// MEASURED, and it is why this exists: `REFLECTION-PLAN-CORE.md` cited
// `REFLECTION-PLAN-ECS.md:1851` at three places for the row carrying
// *"If CORE declines it, EG3 must add the check on its own read path and say
// so"*. That sentence is at `:1853`; `:1851` is a **plausible sibling row of
// the same table** — the `NestedCursorMut` value-model row, scheduled against
// EG4/EG5 — and the census passed over it in silence. The anchor it REPLACED —
// `:1514`, ECS's numbering of the day — was blank and reddened the census. **The repair made the
// citation worse and the gate quieter at the same time**, which is the shape
// this whole campaign keeps finding: an anchor landing on plausible content is
// strictly more dangerous than one out of range, because the reader who follows
// it arrives somewhere that reads like an answer.
//
// The check: a cross-document citation almost always QUOTES the target. Where
// the citing text carries a quoted phrase, that phrase must appear inside the
// cited line range. Where it carries none, the anchor falls back to bounds and
// is COUNTED, so the un-checkable population cannot grow in silence — the same
// discipline `OVER_WAIVED_MAX` applies to waivers.
// ────────────────────────────────────────────────────────────────────────────

/// Reduce a markdown span to comparable text: emphasis and code delimiters
/// dropped, every whitespace run collapsed to one space.
///
/// Applied to BOTH sides, so a quotation written `*"…"*` still matches a target
/// that writes the same words inside `~~…~~` or `**…**` — which is exactly what
/// the measured case does, the target having STRUCK the sentence the citation
/// quotes. `_` is deliberately NOT stripped: these documents use `*` for
/// emphasis, and underscore is an identifier byte here (`install_type_info`).
fn normalize_md(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    for ch in s.chars() {
        if matches!(ch, '*' | '~' | '`' | '\\') {
            continue;
        }
        if ch.is_whitespace() {
            pending_space = !out.is_empty();
        } else {
            if pending_space {
                out.push(' ');
                pending_space = false;
            }
            out.push(ch);
        }
    }
    out
}

/// A quoted phrase must be at least this many normalized characters to be read
/// as a citation of another document's words.
///
/// Below it the corpus is full of scare-quoted TERMS — `"green"`, `"the
/// oracle"` — which name a concept rather than reproduce a sentence, and
/// demanding one of those inside the cited line would red correct anchors.
const MIN_QUOTE_LEN: usize = 24;

/// One doc-to-doc citation whose cited lines do not carry the text it quotes.
///
/// Structured rather than a bare message so `KNOWN_STALE` can name an individual
/// finding instead of pinning a count. A count would let one repaired anchor pay
/// for one freshly broken one — the failure mode this campaign has already
/// measured on `planned_paths`, where the pin moved for the wrong reason.
struct StaleQuote {
    /// Line of the CITING document the anchor sits on.
    citing_line: usize,
    /// File name of the cited document.
    target: String,
    /// The line number the citation gives.
    cited_line: usize,
    /// The rendered report line.
    message: String,
}

/// A literal quotation, and the byte offset in its paragraph at which it opens.
struct Quote {
    at: usize,
    text: String,
}

/// Every quoted phrase in `para`, normalized, that is a LITERAL reproduction of
/// another document's words.
///
/// Three filters, each written after a measured false positive on the incumbent
/// corpus rather than guessed at:
///
/// * **Balance.** An odd number of `"` in the text means the scan is cutting a
///   quotation in half, and every fragment it yields is then the *gap between*
///   two quotations rather than a quotation — measured as fragments beginning
///   with `,` or `;` and carrying whole markdown links. Nothing is extracted in
///   that case; the anchor falls back to bounds and is counted.
/// * **Elision.** `…` / `...` inside a quotation means the citer dropped words,
///   so the phrase does not appear verbatim anywhere and a substring test would
///   red a correct citation. MEASURED on `MESHLET-VIRTUAL-GEOMETRY-PLAN.md:630`
///   and `REFLECTION-PLAN-CORE.md:2610`.
/// * **Interpolation.** `[` marks an editorial insertion (*"three sibling
///   documents [that] say …"*, `REFLECTION-PLAN-ECS.md:2330`) or a markdown
///   link that leaked in; either way the span is not verbatim.
///
/// Curly quotes count as delimiters alongside straight ones: the corpus writes
/// both, and recognising one form alone would silently halve the population
/// checked — and, worse, would leave an odd delimiter count that the balance
/// rule then reads as a misparse.
fn quotations(para: &str) -> Vec<Quote> {
    let marks: Vec<(usize, usize)> = para
        .char_indices()
        .filter(|(_, c)| matches!(c, '"' | '\u{201C}' | '\u{201D}'))
        .map(|(i, c)| (i, c.len_utf8()))
        .collect();
    if !marks.len().is_multiple_of(2) {
        return Vec::new();
    }
    marks
        .chunks(2)
        .filter_map(|pair| {
            let (open, open_len) = pair[0];
            let (close, _) = pair[1];
            let text = normalize_md(para.get(open + open_len..close)?);
            (text.chars().count() >= MIN_QUOTE_LEN
                && text.contains(' ')
                && !text.contains('\u{2026}')
                && !text.contains("...")
                && !text.contains('['))
            .then_some(Quote { at: open, text })
        })
        .collect()
}

/// One paragraph of a document: a maximal run of non-blank lines.
///
/// The paragraph, not the line, is the unit a quotation belongs to. These
/// documents hard-wrap at ~100 columns, so a quoted sentence routinely starts
/// above its citation and finishes below it — MEASURED on all three `:1853`
/// sites, where the quotation spans two lines in every case and *opens* on the
/// line above the anchor in one. A fixed line window cuts those quotations in
/// half; the paragraph does not, which is also what makes the balance filter in
/// [`quoted_fragments`] mean something.
struct Paragraph {
    /// The paragraph's literal quotations, by opening offset.
    quotes: Vec<Quote>,
    /// `(line index, column, paragraph offset)` of EVERY anchor in the
    /// paragraph, in reading order — `.rs` and `.toml` citations included,
    /// because an intervening anchor of any kind ends the previous one's claim
    /// on what follows it. See [`attributable_quotes`].
    anchors: Vec<(usize, usize, usize)>,
}

/// Split a document into paragraphs and index each line to the one containing
/// it. Blank lines belong to no paragraph.
fn build_paragraphs(lines: &[&str]) -> (Vec<Option<usize>>, Vec<Paragraph>) {
    let mut para_of = vec![None; lines.len()];
    let mut paragraphs: Vec<Paragraph> = Vec::new();
    let mut start = 0usize;
    let mut open = false;

    let flush = |start: usize,
                 end: usize,
                 paragraphs: &mut Vec<Paragraph>,
                 para_of: &mut Vec<Option<usize>>| {
        let idx = paragraphs.len();
        let mut anchors = Vec::new();
        // Offsets into the joined paragraph text, so anchors and quotations are
        // comparable in one coordinate system. The join separator is one byte.
        let mut base = 0usize;
        for (off, line) in lines[start..end].iter().enumerate() {
            para_of[start + off] = Some(idx);
            for a in scan_line(line, &docs_dir()).1 {
                anchors.push((start + off, a.col, base + a.col));
            }
            base += line.len() + 1;
        }
        paragraphs.push(Paragraph {
            quotes: quotations(&lines[start..end].join(" ")),
            anchors,
        });
    };

    for (i, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            if open {
                flush(start, i, &mut paragraphs, &mut para_of);
                open = false;
            }
        } else if !open {
            start = i;
            open = true;
        }
    }
    if open {
        flush(start, lines.len(), &mut paragraphs, &mut para_of);
    }
    (para_of, paragraphs)
}

/// [`attributable_quotes`] for an anchor addressed by document line index.
fn quotes_for(
    para_of: &[Option<usize>],
    paragraphs: &[Paragraph],
    line_idx: usize,
    col: usize,
) -> Vec<String> {
    match para_of.get(line_idx).copied().flatten() {
        Some(p) => attributable_quotes(&paragraphs[p], line_idx, col),
        None => Vec::new(),
    }
}

/// The quotations one anchor may be checked against: **those between it and the
/// next anchor in the paragraph.**
///
/// These documents state a citation and then quote it — *"`X.md:N` says «…»"*,
/// *"`X.md:N` lists this against EG3, with a then-live fallback — «…»"* — so a
/// quotation belongs to the anchor most recently written before it, and the
/// next anchor of any kind ends that claim. `.rs` and `.toml` anchors are
/// boundaries too even though they are never subjects here: an intervening code
/// citation takes ownership of the quotation that follows it.
///
/// ⚠️ **The DIRECTION is the whole rule, and the `over_waived` commentary at the
/// end of this file is the reason it was measured rather than assumed** — the
/// identifier pairing is off by one precisely where a document writes the symbol
/// AFTER its anchor. For quotations the convention runs the other way and holds
/// across the whole gated corpus.
///
/// Two attribution rules were tried first. Both are recorded because each was
/// refuted by the corpus rather than by argument:
///
/// * *"one anchor in the paragraph ⇒ every quotation is a candidate"* — FALSE
///   POSITIVE on `REFLECTION-ANALYSIS.md:73`, whose paragraph cites
///   `docs/FEATURE_MAP.md:112` correctly and then, four lines further on and
///   under a `Cargo.toml` anchor of its own, quotes that manifest.
/// * *"equal counts ⇒ pair positionally"* — put `REFLECTION-PLAN-CORE.md:3575`
///   and `:3981`, the two cleanest instances of the very defect this check
///   exists for, into the un-checkable population, because their paragraphs
///   also cite `mod.rs:388` and `registry.rs:87`.
fn attributable_quotes(p: &Paragraph, line_idx: usize, col: usize) -> Vec<String> {
    let Some(rank) = p
        .anchors
        .iter()
        .position(|&(l, c, _)| (l, c) == (line_idx, col))
    else {
        return Vec::new();
    };
    let from = p.anchors[rank].2;
    let to = p
        .anchors
        .get(rank + 1)
        .map_or(usize::MAX, |&(_, _, off)| off);
    p.quotes
        .iter()
        .filter(|q| q.at > from && q.at < to)
        .map(|q| q.text.clone())
        .collect()
}

/// What the document wrote immediately left of an anchor's colon.
///
/// The distinction that matters is **named vs unnamed**, not bound vs unbound. An anchor whose
/// left side names nothing is a continuation and *must* inherit the section's binding — that is
/// how `` `tag_api.rs:130/:200/:89` `` and every wrapped `:N` in these documents work, and it is
/// 722 of the corpus's 1378 colon anchors. An anchor whose left side names *something* is the
/// document stating its own target, and inheriting a different one there is a silent wrong answer.
enum LeftOfAnchor<'a> {
    /// A file-shaped fragment: `mod.rs:46`, `component_registry/tags.rs:134`, and — since the
    /// split form below — `` `REFLECTION-PLAN-BOUNDARY.md`:1292 ``.
    Named(&'a str),
    /// A name-shaped token that is **not** a file name, so the binder cannot turn it into a path:
    /// a document ALIAS, `` `GATES:1316-1321` ``. Counted by
    /// [`aliases_that_inherit_a_binding_are_reported_and_pinned`], then allowed to inherit —
    /// see that test for why refusing would cost a correct check rather than buy one.
    Alias(&'a str),
    /// Nothing name-shaped is written left of the colon: a genuine `:N` continuation.
    Continuation,
}

/// Markup that may sit between a file name and its colon.
///
/// ⚠️ **`~` is deliberately absent and must stay absent**: it is the waiver marker, and admitting
/// it here would make `:N~` parse as a name-terminating delimiter rather than a waived anchor.
const NAME_DELIMITERS: &[u8] = b"`*";

/// The `file.rs` fragment written immediately left of an anchor's colon, classified.
///
/// `// mod.rs:46` yields `Named("mod.rs")`; `(enable_store.rs:259)` yields
/// `Named("enable_store.rs")`; a bare `// :59` yields `Continuation` and the anchor falls back to
/// the section's current target.
///
/// ⚠️ **The SPLIT form — a backtick between the name and the colon — used to yield
/// `Continuation`, and that is how a citation bound to the wrong document and passed.** MEASURED:
/// `REFLECTION-PLAN-ECS.md:2668` writes `` (`REFLECTION-PLAN-BOUNDARY.md`:1292) ``. The backwards
/// walk stopped dead on the backtick, returned nothing, and the anchor inherited the sticky target
/// `docs/OPEN-QUESTIONS.md` — set two lines earlier by an unrelated mention. Both halves were
/// observed before this repair. Mutating the cited number to 3000 reddened the census, and its
/// message *named the inherited document rather than the cited one*. Mutating it to 2000 — **422
/// lines past the end of the 1578-line document the citation actually names** — passed with exit 0,
/// because the file it was silently checked against has 3537 lines. The named target was never
/// opened: not content-checked, not bounds-checked.
///
/// (Those two numbers are written as bare quantities. Writing them beside the document's name would
/// make this comment carry two live citations of a mutation that was reverted — see
/// [`md_citations_in_rust_sources`], which reads this file.)
///
/// One live instance corpus-wide, and it is inherited rather than new. Measured over the nine gated
/// documents: 654 anchors carry a file-shaped run, 669 carry an empty one (true continuations), 53
/// carry a digits-and-`/` run (the `:130/:200` list form), **1** is this split form, and **1** is an
/// alias. The delimiter set is [`NAME_DELIMITERS`] rather than "the backtick" so the next wrapper
/// someone reaches for — `**bold**` — does not reopen the same hole.
fn fragment_before(text: &str, col: usize) -> LeftOfAnchor<'_> {
    let (run, start) = path_run_ending_at(text, col);
    if let Some(c) = classify_run(run) {
        return c;
    }
    // An empty run may mean the name is separated from its colon by markup rather than absent.
    let bytes = text.as_bytes();
    let mut k = start;
    while k > 0 && NAME_DELIMITERS.contains(&bytes[k - 1]) {
        k -= 1;
    }
    if k == start {
        return LeftOfAnchor::Continuation;
    }
    let (run, _) = path_run_ending_at(text, k);
    classify_run(run).unwrap_or(LeftOfAnchor::Continuation)
}

/// The maximal run of path bytes ending at `col`, and where it starts.
///
/// Returns an empty run when `col` is not preceded by path bytes, and also when the walk lands
/// mid-character — `is_path_byte` is ASCII-only, so it stops in front of any multi-byte character,
/// and a slice starting inside one is not a `str`.
fn path_run_ending_at(text: &str, col: usize) -> (&str, usize) {
    let bytes = text.as_bytes();
    let mut start = col;
    while start > 0 && is_path_byte(bytes[start - 1]) {
        start -= 1;
    }
    (text.get(start..col).unwrap_or(""), start)
}

/// `Named` for a file-shaped run, `Alias` for a lettered one that is not, `None` for an empty run
/// or a purely numeric one — the `:130/` tail of a `(a) / (b)` citation list carries no name.
fn classify_run(run: &str) -> Option<LeftOfAnchor<'_>> {
    if run.is_empty() {
        return None;
    }
    if Path::new(run).extension().is_some() {
        return Some(LeftOfAnchor::Named(run));
    }
    run.bytes()
        .any(|b| b.is_ascii_alphabetic())
        .then_some(LeftOfAnchor::Alias(run))
}

/// Resolve a fenced margin-note fragment to a real file, using the section's own binding.
///
/// A fence's margin notes are written relative to the reader's position in the tree, not to the
/// repository root — §9.1 says `system/system.rs` under a section whose **Files:** line is
/// `core/system/`. So the fragment is joined onto the section's base directory and then onto each
/// ancestor of it, and the first join that exists on disk wins. Nothing is guessed: a fragment
/// that lands on no existing file resolves to `None` and its anchor is skipped.
fn resolve_fragment(frag: &str, sticky: Option<&PathBuf>, base: Option<&PathBuf>) -> Option<PathBuf> {
    let root = repo_root();
    // The common case: the note names the section's own file (`mod.rs` under `observers/mod.rs`).
    if let Some(s) = sticky
        && s.ends_with(frag)
        && s.is_file()
    {
        return Some(s.clone());
    }
    let start = match base.or(sticky) {
        Some(p) if p.is_dir() => p.clone(),
        Some(p) => p.parent()?.to_path_buf(),
        None => return None,
    };
    let mut dir = start;
    loop {
        let candidate = dir.join(frag);
        if candidate.is_file() {
            return Some(candidate);
        }
        if dir == root {
            break;
        }
        dir = dir.parent()?.to_path_buf();
        if !dir.starts_with(&root) {
            return None;
        }
    }
    // Last resort: a CRATE-relative fragment. The register corpus writes `boyko_render/src/
    // light.rs:309-310` — the crate name without the `crates/` prefix — and the ancestor walk
    // above cannot reach it from a section whose last path mention was another document.
    // Measured: this is the only anchor `gaia/PENDING-SYNTAX-PLAN.md` has, and without this join
    // that document extracted ZERO and tripped the per-document non-emptiness assert.
    let by_crate = root.join("crates").join(frag);
    by_crate.is_file().then_some(by_crate)
}

/// Every source file a bare fragment could name, walked once per process.
///
/// `target/` and `.git/` are excluded: they hold generated copies of tree files, and a fragment
/// matching both the source and its build artefact would be reported ambiguous for no reason.
fn repo_files() -> &'static Vec<PathBuf> {
    static FILES: std::sync::OnceLock<Vec<PathBuf>> = std::sync::OnceLock::new();
    FILES.get_or_init(|| {
        let mut out = Vec::new();
        let mut stack = vec![repo_root()];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in rd.flatten() {
                let p = entry.path();
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if p.is_dir() {
                    if name != "target" && name != ".git" && name != "node_modules" {
                        stack.push(p);
                    }
                } else {
                    out.push(p);
                }
            }
        }
        out
    })
}

/// Resolve a bare fragment — `enable_tag_api.rs`, `component_registry/tags.rs` — to **the one**
/// file in the tree whose path ends with it.
///
/// # Why this exists
///
/// The three navigation documents anchor a file once and list members under it, which is what the
/// sticky binding models. The five reflection planning documents do the opposite: their tables give
/// **one row per claim**, and each row names its own file by a relative fragment —
/// `` `enable_tag_api.rs:60`, `component_registry/tags.rs:134`, `:155` ``. A fragment is not a
/// resolvable path mention, so before this existed every such anchor fell through to the section's
/// last `crates/...` path. Measured over the five documents at the moment they entered
/// `GATED_DOCS`: **634 citations, of which 53 were rooted `crates/...`** — the only form that bound
/// correctly — **303 bare fragments and 258 bare `:N` continuations**. The arming run reported 150
/// "stale" anchors of 231, and classifying them showed the majority were not stale at all: they
/// were checked against a file the document never named. `REFLECTION-PLAN-ECS.md:75` cites
/// `component_registry/mod.rs:918` and was reported ``past end of file (263 lines)`` against
/// `crates/boyko_ecs/Cargo.toml`.
///
/// # Nothing is guessed
///
/// Zero matches and two-or-more matches both return `None`, and the caller then **skips** the
/// anchor rather than falling back to the sticky binding. Falling back is what produced the
/// misbindings, so an unresolvable fragment must cost coverage, never correctness. A bare `mod.rs`
/// is ambiguous in this tree by construction and is skipped every time; the count is printed and
/// pinned by [`unbindable_fragments_are_reported_and_pinned`] so it cannot grow unnoticed.
///
/// The suffix must land on a path separator, so `tags.rs` never matches `component_tags.rs`.
fn resolve_unique_fragment(frag: &str) -> Option<PathBuf> {
    let mut found: Option<&PathBuf> = None;
    for p in repo_files() {
        if path_ends_with_fragment(p, frag) {
            if found.is_some() {
                // Ambiguous: two files in the tree end with this fragment.
                return None;
            }
            found = Some(p);
        }
    }
    found.cloned()
}

/// Does `path` end with `frag` on a **path-segment boundary**?
///
/// `ends_with` alone is not enough and the difference is not academic: `tags.rs` is a suffix of
/// `component_tags.rs`, so a plain suffix test would bind one file's citations to the other and
/// report line numbers from a file the document never named.
fn path_ends_with_fragment(path: &Path, frag: &str) -> bool {
    let s = path.to_string_lossy().replace('\\', "/");
    let needle = frag.replace('\\', "/");
    match s.len().checked_sub(needle.len()) {
        Some(0) => s == needle,
        Some(k) => s.as_bytes()[k - 1] == b'/' && s.ends_with(&needle),
        None => false,
    }
}

/// How strongly one anchor was checked. Printed as a per-document decomposition so the module
/// doc's claim about the gate's reach is re-derived from the gate rather than remembered.
///
/// The five are exhaustive and disjoint, so they sum to the anchor count.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AnchorClass {
    /// Written `~`: neither shape nor identity is checked, only that line N is inside the file.
    Waived,
    /// The line's backticked symbols did not pair one-to-one with its anchors, so no symbol was
    /// attributed to this anchor. Definition-shape only.
    Unpaired,
    /// A symbol paired, but the cited file declares no item of that name, so claiming identity
    /// would red a correct anchor (the `impl From<…> for …` shape). Definition-shape only.
    PairedUndeclared,
    /// Definition-shape *and* identity: line N must contain the symbol paired with this anchor.
    Identity,
    /// The cited file is PROSE — a `.md` document — so only the in-file bounds check applies.
    ///
    /// ⚠️ This is a narrowing of the predicate, not a waiver, and it is derived from the target's
    /// extension rather than written at a site. Markdown has no definitions, so
    /// `looks_like_definition` answers `false` for every line of a document and reported all of
    /// them stale: the first widened run charged `` `../OPEN-QUESTIONS.md:389` `` and a dozen more
    /// like it as "not a definition", which is a statement about the checker, not about the
    /// citation. The class that DOES rot in a cross-document citation — a line number past the end
    /// of a document that shrank — is still caught, and was: `gaia/CAMPAIGN.md` line 1262 in a file
    /// of 339 lines. The example is written WITHOUT the `:N` form on purpose: it names a DEAD
    /// citation, and on the merged tree `docs/gaia/CAMPAIGN.md` exists, so the colon spelling
    /// would make this sentence the very rot it describes — this census reads `.rs` sources and
    /// flagged it. (On the reflection lane the file was absent, so it bound to nothing and was
    /// counted in the other column instead; the A6 merge is where the two met.)
    /// Counted separately so a corpus that quietly became all-prose cannot look
    /// well-checked.
    Prose,
}

/// Everything one document yields, after fences and ignored lines are removed.
struct DocScan {
    mentions: usize,
    anchors: usize,
    /// Anchor count split by [`AnchorClass`], in declaration order.
    classes: [usize; 5],
    path_violations: Vec<String>,
    anchor_violations: Vec<String>,
    /// Anchors accepted on a non-definition line because the document is in [`EVIDENCE_DOCS`].
    /// Printed, so the strength given up by that genre flag is a number and not a shrug.
    evidence_relaxed: usize,
    /// Path mentions on a `<!-- doc-path-planned -->` line: named as deliverables, not yet on disk.
    planned_paths: Vec<String>,
    /// Lines whose `<!-- doc-path-planned -->` marker waives **nothing** — every path written on
    /// them is already on disk.
    ///
    /// ⚠️ **This is the half of the marker's contract that used to stop being observable at the
    /// exact moment the work succeeded.** [`planned_paths`](DocScan::planned_paths) is filled only
    /// inside the *missing-file* branch, so once a deliverable lands, its marker becomes invisible
    /// to the scan: the count decrements because the FILE now exists, not because the marker came
    /// off, and putting the marker back leaves this census green. MEASURED 2026-08-27 on
    /// `REFLECTION-PLAN-ECS.md` §7's `ecs_alloc.rs` line — restored marker, still 8/8, exit 0. A rung
    /// whose gate is *"the marker comes off AND the pin decrements"* was therefore gating one fact
    /// and reporting two.
    ///
    /// A line carrying the marker and **no** path mention is not a subject: the plans discuss the
    /// marker in prose, in backticks, and that prose waives nothing by construction.
    stale_planned: Vec<String>,
    /// Anchors written with a file fragment that resolves to no single file in the tree, and were
    /// therefore skipped rather than checked against a file the document did not name.
    unbindable: Vec<String>,
    /// Waived anchors whose cited line is nevertheless definition-shaped.
    ///
    /// ⚠️ A `~` says "this line is deliberately not the definition of that symbol", and it waives
    /// *both* the shape and the identity assertion. When the cited line passes the shape test
    /// anyway, the waiver is buying nothing on that axis and is giving up a check that would have
    /// held — a silently weakened assertion, which is the exact defect class the campaign this
    /// gate serves keeps finding. It is REPORTED and pinned, not failed: a definition-shaped line
    /// can still be the wrong definition, so an over-waiver is a smell rather than a proof.
    over_waived: Vec<String>,
    /// Doc-to-doc anchors whose citing text quotes the target, and whose cited lines do NOT
    /// contain that quotation. A FAILURE — see the content-check banner above `normalize_md`.
    doc_quote_violations: Vec<StaleQuote>,
    /// Doc-to-doc anchors the content check could not reach, because the citing text quotes
    /// nothing long enough to look for. Bounds-checked only, and PINNED, so the un-checkable
    /// population cannot grow in silence.
    doc_unquoted: Vec<String>,
    /// Every doc-to-doc anchor this document writes — the DENOMINATOR of which
    /// [`doc_unquoted`](DocScan::doc_unquoted) is the un-checkable part.
    ///
    /// ⚠️ **Without it the only published figures were the caps, and a cap reads as a small
    /// residue rather than as a share.** `UNQUOTED_MAX` sits at the live count for every document
    /// with zero headroom, which is good discipline and says nothing at all about how much of the
    /// population it is: at this revision the bounds-only anchors are the MAJORITY. A ledger that
    /// publishes the numerator and not the denominator is the same defect as a prose count that
    /// nobody re-measures, so the split is printed by the gate instead of being written down.
    doc_to_doc: usize,
    /// Anchors written behind a [`LeftOfAnchor::Alias`] — the document named its target with a
    /// token the binder cannot resolve to a file, so the anchor inherited the section's binding.
    ///
    /// Reported and pinned rather than failed or skipped. See
    /// [`aliases_that_inherit_a_binding_are_reported_and_pinned`].
    alias_bound: Vec<String>,
    /// Lines carrying [`IGNORE_MARKER`], which drops the whole line before any anchor on it is
    /// scanned. The strongest opt-out in this file, and until now the only one with no ledger.
    ignored_lines: Vec<String>,
    /// What each [`IGNORE_MARKER`] line would have reported had it been scanned.
    ///
    /// ⚠️ **The line ledger above counts LINES, and a marked line's CONTENT is unbounded.**
    /// MEASURED 2026-08-28 on `REFLECTION-PLAN-CORE.md:2128`: appending a real anchor violation
    /// *and* a real path violation to an already-marked line leaves the suite at exit 0 with every
    /// printed number unmoved, while the same citation on an unmarked line reds at exit 101. So the
    /// marker is a permanently open slot — a ceiling on how many lines carry it says nothing about
    /// how much each one silences. [`PLANNED_MARKER`] has had both halves of its contract gated
    /// since the revision that added [`DocScan::stale_planned`]; this is the second half for the
    /// strictly stronger opt-out.
    ///
    /// Measured by re-scanning the marked line with the marker removed, on a synthetic one-line
    /// document prefixed with a link that restores the sticky binding the line had in place. The
    /// binding is restored on the SAME line on purpose: the marked line is measured as it would be
    /// read, not as a fresh document that has bound nothing.
    silenced: Vec<String>,
    /// Marked lines whose re-scan reports nothing — the marker waives nothing and is a slot left
    /// open for the next citation written on that line. The exact analogue of
    /// [`stale_planned`](DocScan::stale_planned), and failed by the same test that pins the count.
    stale_ignored: Vec<String>,
    /// Continuation anchors that would have inherited a **document** target bound on an EARLIER
    /// line. REFUSED — the anchor is not checked at all — and pinned at zero per document.
    ///
    /// See [`continuations_do_not_inherit_a_document_across_a_line_boundary`] for the measurement
    /// that chose refusal over counting, and [`cross_line_src`](DocScan::cross_line_src) for the
    /// denominator that made the choice safe.
    cross_line_doc: Vec<String>,
    /// The same shape onto a **source** file: `**File:** foo.rs` on one line and `(582)` on the
    /// next, which is the binding this gate was built for. Allowed, and counted so that the
    /// refusal above is read against its denominator rather than as a rule about continuations.
    cross_line_src: usize,
    /// Doc-to-doc anchors whose quotation is absent from the BOUND target and present in the
    /// CITING document at the very coordinates the anchor names.
    ///
    /// ⚠️ **This is the false-negative class the misbinding produced, and it is reported here
    /// rather than in [`doc_unquoted`](DocScan::doc_unquoted) because the two are opposite
    /// verdicts.** `doc_unquoted` says *"the instrument could not reach this anchor"*; an anchor
    /// whose words are at its own numbers in its own document is one the instrument reached and
    /// answered wrongly, because it was pointed at the wrong file. Left in the un-checkable
    /// population it reads as discipline; named here it reads as what it is.
    doc_misbound: Vec<String>,
}

impl DocScan {
    fn count(&self, class: AnchorClass) -> usize {
        self.classes[class as usize]
    }
}

/// The normalized text a `:N` / `:N-M` citation covers, plus the byte length of the part of it the
/// citation actually names.
///
/// ⚠️ The window runs PAST the cited range by `READ_AHEAD` lines and the reason that is not a
/// loophole is the returned length: these documents hard-wrap at ~100 columns, so a quoted sentence
/// almost never fits on the one line that cites it, and matching against the cited line alone
/// reported eleven wrapped-but-correct citations as stale (MEASURED). What a line citation claims is
/// where the material BEGINS, so a match is accepted only when it begins before `cited_len`.
///
/// Extracted so the two callers cannot drift: the target-side check that a citation carries the
/// words it quotes, and the citing-side check for [`DocScan::doc_misbound`] that asks the same
/// question of the citing document at the same coordinates.
fn cited_window(src: &[String], start: usize, end: Option<usize>) -> (String, usize) {
    const READ_AHEAD: usize = 8;
    if start == 0 || start > src.len() {
        return (String::new(), 0);
    }
    let last = end.unwrap_or(start).max(start).min(src.len());
    let mut window = String::new();
    let mut cited_len = 0usize;
    for (i, l) in src[start - 1..(last + READ_AHEAD).min(src.len())]
        .iter()
        .enumerate()
    {
        let n = normalize_md(l);
        if !n.is_empty() {
            if !window.is_empty() {
                window.push(' ');
            }
            window.push_str(&n);
        }
        if start + i <= last {
            cited_len = window.len();
        }
    }
    (window, cited_len)
}

/// Check one bound anchor, in whichever form it was written — the three forms (`file.rs:N`,
/// `(:N)`, bare `(N)`) differ only in how `scan_line` finds them, never in what is asserted here.
/// Returns the strength at which it was checked, or `None` when the cited file was unreadable and
/// the anchor was therefore not counted; the caller keeps the denominator so the two call sites
/// cannot drift apart.
#[allow(clippy::too_many_arguments)]
fn check_anchor(
    doc: &str,
    lineno: usize,
    raw: &str,
    target: &Path,
    anchor: &Anchor,
    expect_ident: Option<&str>,
    evidence: bool,
    anchor_quotes: &[String],
    file_lines: &mut BTreeMap<PathBuf, Option<Vec<String>>>,
    md_text: &mut BTreeMap<PathBuf, String>,
    out: &mut Vec<String>,
    over_waived: &mut Vec<String>,
    relaxed: &mut usize,
    doc_quote_violations: &mut Vec<StaleQuote>,
    doc_unquoted: &mut Vec<String>,
    doc_misbound: &mut Vec<String>,
) -> Option<AnchorClass> {
    let entry = file_lines.entry(target.to_path_buf()).or_insert_with(|| {
        std::fs::read_to_string(target)
            .ok()
            .map(|s| s.lines().map(str::to_string).collect())
    });
    let src = entry.as_ref()?;

    // Classified before the bounds test, so the decomposition describes the reach of the gate
    // rather than which anchors happen to be failing today.
    let is_prose = target
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("md"));
    let class = if anchor.shape_waived {
        AnchorClass::Waived
    } else if is_prose {
        AnchorClass::Prose
    } else if let Some(ident) = expect_ident {
        if src
            .iter()
            .any(|l| leading_decl_name(l).as_deref() == Some(ident))
        {
            AnchorClass::Identity
        } else {
            AnchorClass::PairedUndeclared
        }
    } else {
        AnchorClass::Unpaired
    };

    if anchor.line_no > src.len() {
        out.push(format!(
            "  {doc}:{lineno}  `{raw}:{}` is past end of file ({} lines)",
            anchor.line_no,
            src.len()
        ));
        return Some(class);
    }
    // A range is a claim about extent; an end below its start is not a stale number but an
    // incoherent one, and the shape test on the first line cannot see it.
    if let Some(end) = anchor.range_end {
        if end < anchor.line_no {
            out.push(format!(
                "  {doc}:{lineno}  `{raw}:{}-{end}` ends before it starts",
                anchor.line_no
            ));
        } else if end > src.len() {
            out.push(format!(
                "  {doc}:{lineno}  `{raw}:{}-{end}` ends past end of file ({} lines)",
                anchor.line_no,
                src.len()
            ));
        }
    }

    let ext = target
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    let src_line = &src[anchor.line_no - 1];

    // ── Doc-to-doc: CONTENT, not just bounds. ───────────────────────────────
    //
    // Placed BEFORE the `Waived` early return on purpose. A `~` waives the
    // definition-SHAPE assertion, and for a `.md` target there is no shape
    // assertion to waive — `looks_like_definition` returns `true` for every
    // non-`.rs` extension by construction. Letting `~` suppress this check
    // would hand every doc-to-doc citation a one-character opt-out of the only
    // thing that is ever asserted about it.
    if ext.eq_ignore_ascii_case("md") {
        // The window runs past the cited range; see `cited_window` for why that
        // is not a loophole. `REFLECTION-PLAN-ECS.md:1851` still reds against the
        // sentence at `:1853`, two lines inside its own read-ahead.
        let (target_window, cited_len) = cited_window(src, anchor.line_no, anchor.range_end);
        // ⚠️ **A quotation the cited document does not contain AT ALL is evidence
        // about the ATTRIBUTION, not about the line number, and reporting it
        // would be reporting the wrong thing.** The paragraph rule owns a
        // quotation to the anchor most recently written before it, and in a long
        // bulleted paragraph that anchor is sometimes not the one the quotation
        // belongs to. MEASURED: five of the thirteen first reports were this —
        // `REFLECTION-PLAN-CORE.md:3639` cites GATES for *"the paths and fixture
        // names it reserves"* and the paragraph goes on to quote something else
        // entirely, three sentences later, with no anchor in between.
        //
        // So the assertion this gate makes is exactly, and only: **you quoted
        // this document, and the line you named is not where those words are.**
        // A quotation that appears nowhere in the target is a different defect
        // (a paraphrase, or a wrong document) that this instrument cannot tell
        // apart from a mis-attribution, and it falls back to bounds and is
        // counted rather than guessed at.
        let full = md_text
            .entry(target.to_path_buf())
            .or_insert_with(|| normalize_md(&src.join(" ")));
        let present: Vec<&String> = anchor_quotes
            .iter()
            .filter(|f| full.contains(f.as_str()))
            .collect();

        if present.is_empty() {
            // ⚠️ **"Bounds only" is the right verdict for an anchor the instrument could not
            // reach, and the WRONG one for an anchor it reached and misbound.** Before this
            // branch existed, `REFLECTION-PLAN-CORE.md:435` cited `(:344-349)` for a fenced
            // sketch that is in CORE, inherited `REFLECTION-ANALYSIS.md` from the previous line,
            // and was filed as un-checkable — inside the population the census publishes as
            // "bounds-only, pinned with zero headroom, which is real discipline".
            //
            // So ask the citing document the same question at the same coordinates. A hit is not
            // proof, but it is the one piece of evidence that separates the two verdicts, and it
            // is a strictly narrower test than "the citing document contains these words" — which
            // is vacuous, since the quotation was read out of that document in the first place.
            //
            // Read uncached, and only when the citing text quoted something at all: the common
            // unquoted anchor carries no quotation and never reaches the filesystem here.
            //
            // ⚠️ **STATED NON-COVERAGE — `self_path != target` exempts a SELF-CITATION by design,
            // and the corpus holds two.** A document citing its own lines has no second document
            // to have been misbound to, so the evidence this branch collects is the same read the
            // target side already made: every self-citation whose quotation merely wraps past
            // `cited_len` would be reported as a misbinding it cannot be. The two live ones both
            // point at C11's weaker-subject rule from inside the file that states it —
            //     REFLECTION-PLAN-CORE.md:512
            // and
            //     REFLECTION-PLAN-CORE.md:2575
            // — and both resolve correctly. Neither reaches this branch today for a SECOND and
            // independent reason: neither quotes anything, so `anchor_quotes` is empty and both
            // land in the bounds-only ledger with the other 28 CORE carries. Two reasons pointing
            // the same way is exactly the shape that hides a gate that cannot fail, so it is
            // written down: what covers a self-citation is the ordinary target-side quote check
            // above, which reads the same file either way, and NOTHING here.
            let self_path = docs_dir().join(doc);
            if !anchor_quotes.is_empty()
                && self_path != target
                && let Ok(text) = std::fs::read_to_string(&self_path)
            {
                let self_src: Vec<String> = text.lines().map(str::to_string).collect();
                let (self_window, self_cited) =
                    cited_window(&self_src, anchor.line_no, anchor.range_end);
                if let Some(hit) = anchor_quotes
                    .iter()
                    .find(|f| self_window.find(f.as_str()).is_some_and(|p| p < self_cited))
                {
                    doc_misbound.push(format!(
                        "  {doc}:{lineno}  `{raw}:{}` was checked against `{raw}`, which does not \
                         contain the quoted words -- but THIS document carries them at its own \
                         `:{}`.\n\x20     quoted here: \"{hit}\"",
                        anchor.line_no, anchor.line_no
                    ));
                }
            }
            doc_unquoted.push(format!(
                "  {doc}:{lineno}  `{raw}:{}` quotes nothing this document contains; bounds only",
                anchor.line_no
            ));
        } else if !present
            .iter()
            .any(|f| target_window.find(f.as_str()).is_some_and(|p| p < cited_len))
        {
            doc_quote_violations.push(StaleQuote {
                citing_line: lineno,
                target: raw.rsplit('/').next().unwrap_or(raw).to_string(),
                cited_line: anchor.line_no,
                message: format!(
                    "  {doc}:{lineno}  `{raw}:{}` does not carry the quoted text.\n\
                     \x20     quoted here: \"{}\"\n\
                     \x20     that line reads: {}",
                    anchor.line_no,
                    present
                        .iter()
                        .map(|f| f.as_str())
                        .collect::<Vec<_>>()
                        .join("\" / \""),
                    src_line.trim()
                ),
            });
        }
    }

    // ORDER (A6 merge): the lane's doc-to-doc CONTENT check runs BEFORE the line's `Prose`
    // bounds-only early return, or the return would delete it — `Prose` is exactly the `.md`
    // class the content check owns. Both assertions therefore still run: the words must be at
    // the line named (lane), and then nothing further is asserted about a prose target (line).
    if class == AnchorClass::Prose {
        // Bounds only: the past-EOF and range-coherence checks above have already run.
        return Some(class);
    }
    if class == AnchorClass::Waived {
        // `~` says "this line is deliberately not the definition of that symbol", which waives
        // the identity claim as well as the shape one — the two assert the same thing.
        //
        // But record the ones that did not need it: a waiver on a definition-shaped line gives up
        // an assertion that would have held. See `DocScan::over_waived`.
        if looks_like_definition(src_line, ext) {
            over_waived.push(format!(
                "  {doc}:{lineno}  `{raw}:{}~` is waived, yet that line is definition-shaped: {}",
                anchor.line_no,
                src_line.trim()
            ));
        }
        return Some(class);
    }
    let shaped = looks_like_definition(src_line, ext);
    if !shaped {
        // ⚠️ In an evidence document the NAME test goes with the shape test, and that pairing is
        // measured rather than assumed. A register cites *the line that states the fact* — "the
        // `any_changed_since` doc comment (`:386-389`) claims the scan is bounded" — so the cited
        // line is precisely NOT the declaration, and demanding it contain the symbol reports a
        // correct citation as stale. Two of the three findings that survived the genre flag were
        // exactly that shape (`worker_main` against a doc-comment line, `any_changed_since`
        // against its own doc comment); the third, `push_task` against
        // `pub(crate) fn unmark_idle(`, lands on a DECLARATION and is real. So identity is kept
        // wherever the citation claims a declaration, and dropped wherever it claims evidence.
        if evidence {
            *relaxed += 1;
            return Some(class);
        }
        out.push(format!(
            "  {doc}:{lineno}  `{raw}:{}` is not a definition; that line reads: {}",
            anchor.line_no,
            src_line.trim()
        ));
    } else if class == AnchorClass::Identity
        // Identity is claimed only against a file that declares the symbol (that is what
        // separates `Identity` from `PairedUndeclared` above). An ordinary English word that
        // happens to be backticked — "snapshot `last`'s bit" — is prose next to a citation
        // rather than a label for it, and pairing it anyway would red a correct anchor.
        && let Some(ident) = expect_ident
        && !contains_word(src_line, ident)
    {
        // Shape alone cannot tell one definition from another: it passes just as happily on the
        // wrong `pub fn`. The identifier the doc prints beside the anchor is the identity claim.
        out.push(format!(
            "  {doc}:{lineno}  `{raw}:{}` does not define `{ident}`; that line reads: {}",
            anchor.line_no,
            src_line.trim()
        ));
    }
    Some(class)
}

/// The directory a document's relative link targets are written from — its own, not `docs/`.
/// `GATED_DOCS` entries may name a subdirectory (`gaia/DECISIONS.md`), so this is the parent of
/// the resolved document path rather than a constant.
fn doc_dir_of(doc: &str) -> PathBuf {
    docs_dir()
        .join(doc)
        .parent()
        .expect("invariant: a gated document path always has a parent directory")
        .to_path_buf()
}

fn scan_doc(doc: &str) -> DocScan {
    let path = docs_dir().join(doc);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("gated internal doc {} is unreadable: {e}", path.display()));
    scan_text(doc, &text)
}

/// The scan itself, over text rather than a path.
///
/// Split out so a test can hand it a document it constructed. That is not a convenience: the
/// ignore-marker ledger added in this revision has to prove that a marker on a line carrying a
/// REAL violation both silences the violation and shows up in the count, and the only way to
/// assert that without waiting for someone to write one is to build the line.
fn scan_text(doc: &str, text: &str) -> DocScan {
    // The citing document's own directory: a `GATED_DOCS` entry may name a subdirectory
    // (`gaia/DECISIONS.md`), so a relative mention resolves against it, not against `docs/`.
    let doc_dir = doc_dir_of(doc);
    let mut mentions = 0usize;
    let mut anchors = 0usize;
    let mut classes = [0usize; 5];
    let mut path_violations = Vec::new();
    let mut anchor_violations = Vec::new();
    let mut over_waived = Vec::new();
    let mut evidence_relaxed = 0usize;
    let evidence = EVIDENCE_DOCS.contains(&doc);
    let mut unbindable = Vec::new();
    let mut planned_paths = Vec::new();
    let mut stale_planned = Vec::new();
    let mut doc_quote_violations = Vec::new();
    let mut doc_unquoted = Vec::new();
    let mut doc_to_doc = 0usize;
    let mut alias_bound = Vec::new();
    let mut ignored_lines = Vec::new();
    let mut silenced = Vec::new();
    let mut stale_ignored = Vec::new();
    let mut cross_line_doc = Vec::new();
    let mut cross_line_src = 0usize;
    let mut doc_misbound = Vec::new();

    // The citing side of a doc-to-doc quotation wraps across source lines, so the content check
    // reads a PARAGRAPH rather than the anchor's own line. Collected once here — `text.lines()` is
    // walked again below and the paragraph index needs random access into it.
    let doc_lines: Vec<&str> = text.lines().collect();
    let (para_of, paragraphs) = build_paragraphs(&doc_lines);

    // Cache of file contents, so a document citing one file 15 times reads it once.
    let mut file_lines: BTreeMap<PathBuf, Option<Vec<String>>> = BTreeMap::new();
    // Normalized whole-file text of each `.md` target, for the "does this document contain the
    // quotation at all" discriminator in `check_anchor`.
    let mut md_text: BTreeMap<PathBuf, String> = BTreeMap::new();
    // The sticky anchor target: the last file-shaped mention seen, reset at each heading.
    let mut current: Option<(String, PathBuf, bool, usize)> = None;
    // The source-tree path a fenced margin-note fragment resolves against. Unlike `current` it
    // accepts directories (a section's `**Files:** [core/system/]` is exactly what `system/
    // system.rs` inside the fence below it is written relative to) and it survives sub-headings,
    // because `### 9.2` inherits the file list of `## 9`.
    let mut fence_base: Option<PathBuf> = None;
    let crates_root = repo_root().join("crates");
    let mut in_fence = false;
    // Inside a fence: the file its margin notes currently point into, and the line that named it.
    // The line is carried for the same reason `current` carries one — see `DocScan::cross_line_doc`.
    let mut fence_target: Option<(String, PathBuf, usize)> = None;

    // The register's own retraction convention: text inside `~~ … ~~` is STRUCK, and a struck
    // citation is the record of a claim that was withdrawn, not a live claim. It keeps its old
    // line number on purpose (2026-09-10: fourteen ruled ballots and one refuted mechanism were
    // struck in place with dated pointers rather than deleted). Checking a struck anchor for
    // identity would red on every honest retraction, so struck spans are blanked before the
    // scan — blanked, not removed, so every line number below still means what it means.
    let text = blank_struck(text);
    let text = text.as_str();

    for (idx, line) in text.lines().enumerate() {
        let lineno = idx + 1;
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            if in_fence {
                // A fence opens on its section's file, so a bare `:N` before the first margin
                // note still has a target.
                // Bound on the FENCE's line, which is always earlier than any margin note inside
                // it: a `:N` seeded this way is a cross-line inheritance by construction.
                fence_target = current
                    .as_ref()
                    .filter(|(_, _, exists, _)| *exists)
                    .map(|(raw, p, _, _)| (raw.clone(), p.clone(), lineno));
            }
            continue;
        }
        if line.contains(IGNORE_MARKER) {
            // Counted BEFORE the skip, because the skip is total: the line's anchors, its path
            // mentions and its quotations all cease to exist for every check in this file.
            ignored_lines.push(format!("  {doc}:{lineno}  {}", line.trim()));
            // And measured, because counting the lines bounds nothing: see `DocScan::silenced`.
            // Only a sticky target that EXISTS is restored: a dead one would be written into the
            // synthetic line as a link and counted as a path violation the marker never silenced.
            let would = silenced_by(
                doc,
                line,
                current
                    .as_ref()
                    .filter(|(_, _, exists, _)| *exists)
                    .map(|(_, p, _, _)| p),
            );
            if would.is_empty() {
                stale_ignored.push(format!(
                    "  {doc}:{lineno}  `{IGNORE_MARKER}` silences nothing on this line",
                ));
            }
            for w in would {
                silenced.push(format!("  {doc}:{lineno}  {w}"));
            }
            continue;
        }
        // A deliverable the plan has not built yet is not a dead path. See `PLANNED_MARKER`.
        let planned = line.contains(PLANNED_MARKER);

        if in_fence {
            // Paths are not checked inside a fence — an ASCII tree or an example command is not
            // a claim that a file exists — but the margin notes ARE line claims, and they rot
            // exactly like the prose ones.
            let (_, line_anchors) = scan_line(line, &doc_dir);
            if line_anchors.is_empty() {
                continue;
            }
            // A fence carries no backticks, so its identity claim is the pseudo-declaration the
            // margin note sits on. Only when that note is the line's single anchor, so the same
            // one-to-one rule holds as outside.
            let decl = (line_anchors.len() == 1)
                .then(|| leading_decl_name(line))
                .flatten();
            for anchor in &line_anchors {
                match fragment_before(line, anchor.col) {
                    LeftOfAnchor::Named(frag) => {
                        let from = current.as_ref().map(|c| &c.1);
                        // Unresolvable ⇒ REFUSED, as outside. See `fenced_resolve`.
                        let Some(found) = fenced_resolve(frag, from, fence_base.as_ref()) else {
                            unbindable.push(fenced_unbindable(doc, lineno, frag, anchor.line_no));
                            continue;
                        };
                        fence_target = Some((frag.to_string(), found, lineno));
                    }
                    LeftOfAnchor::Alias(alias) => alias_bound.push(format!(
                        "  {doc}:{lineno}  `{alias}:{}` names its target by an alias; the anchor \
                         inherits the fence's binding ({})",
                        anchor.line_no,
                        fence_target
                            .as_ref()
                            .map_or("none", |(raw, _, _)| raw.as_str())
                    )),
                    // ⚠️ **This arm was an EMPTY BLOCK, and that made the fence the one place a
                    // bare `:N` could inherit a DOCUMENT with neither the refusal nor the count
                    // the non-fenced arm gives it.** `fence_target` is seeded from the sticky
                    // binding at the fence's opening line, and that binding may be a `.md`
                    // document, so the hole was reachable — it was merely unoccupied. MEASURED
                    // 2026-08-28: a bare in-bounds `:N` inside a fence under a document mention
                    // landed in the bounds-only ledger with the cap-0 refusal never moving, and a
                    // bare source-file `:N` inside a fence left BOTH the per-document and the
                    // total continuation counts unchanged. Same rule as outside from now.
                    LeftOfAnchor::Continuation => {
                        if let Some((sraw, spath, set_line)) = fence_target.as_ref()
                            && *set_line != lineno
                        {
                            if is_md_target(spath) {
                                cross_line_doc.push(format!(
                                    "  {doc}:{lineno}  `:{}` inherits `{sraw}`, named on line \
                                     {set_line}; write the document name beside the anchor",
                                    anchor.line_no
                                ));
                                continue;
                            }
                            cross_line_src += 1;
                        }
                    }
                }
                let Some((raw, target, _)) = fence_target.as_ref() else {
                    continue;
                };
                if let Some(class) = check_anchor(
                    doc,
                    lineno,
                    raw,
                    target,
                    anchor,
                    decl.as_deref(),
                    evidence,
                    &quotes_for(&para_of, &paragraphs, idx, anchor.col),
                    &mut file_lines,
                    &mut md_text,
                    &mut anchor_violations,
                    &mut over_waived,
                    &mut evidence_relaxed,
                    &mut doc_quote_violations,
                    &mut doc_unquoted,
                    &mut doc_misbound,
                ) {
                    anchors += 1;
                    classes[class as usize] += 1;
                    if is_md_target(target) {
                        doc_to_doc += 1;
                    }
                }
            }
            continue;
        }

        if trimmed.starts_with('#') && !trimmed.starts_with("#[") {
            current = None;
            if trimmed.bytes().take_while(|b| *b == b'#').count() <= 2 {
                fence_base = None;
            }
        }

        let (line_mentions, line_anchors) = scan_line(line, &doc_dir);
        // A marker on a line whose every path is already on disk waives nothing. See
        // `DocScan::stale_planned` for why this is checked here and not in the branch above: the
        // branch above can only see a marker while the file is still missing.
        if planned && !line_mentions.is_empty() && line_mentions.iter().all(|m| m.resolved.exists())
        {
            stale_planned.push(format!(
                "  {doc}:{lineno}  `{PLANNED_MARKER}` waives nothing -- every path on the \
                 line exists: {}",
                line_mentions
                    .iter()
                    .map(|m| format!("`{}`", m.raw))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        // Positional pairing of the line's backticked symbols to its anchors, and only when the
        // two counts match exactly: `add_tag` / `remove_tag` / `has_tag` against `:130/:200/:89`
        // pairs one-to-one, in order, whichever side of the anchors the symbols are written on.
        // Any other ratio means the line mixes citation labels with prose, and a positional guess
        // there would red correct anchors — so the identity claim is simply not made.
        let idents = backticked_idents(line);
        let paired = idents.len() == line_anchors.len() && !idents.is_empty();

        // Walk mentions and anchors together in column order so an anchor binds to the mention
        // to its left.
        //
        // The `current` sticky binding tracks it, and a fragment written directly in front of
        // an anchor overrides it (the `LeftOfAnchor::Named` arm below). ⚠️ Without that override,
        // resolvable as a path binds to whatever was cited last and is judged against a file it
        // never named — the largest false class in the first widened run. Measured on
        // OPEN-QUESTIONS.md:497-498, where `solver/colored.rs:2667`, `soft/colored.rs:1031`,
        // `resources.rs:1688` and `:1760` were all charged to a `worker.rs` link from fourteen
        // lines above and reported "past end of file (1397 lines)" — four findings, none of them
        // about any of those four files. A fragment that DOES resolve rebinds (the fence rule,
        // applied outside a fence); one that does not CLEARS the binding, so the anchor and any
        // bare `:N` continuation after it are skipped rather than misattributed.
        let mut mi = 0usize;
        for (ai, anchor) in line_anchors.iter().enumerate() {
            while mi < line_mentions.len() && line_mentions[mi].col < anchor.col {
                let m = &line_mentions[mi];
                mentions += 1;
                if !m.resolved.exists() {
                    let bucket = if planned { &mut planned_paths } else { &mut path_violations };
                    bucket.push(format!(
                        "  {doc}:{lineno}  path does not exist: `{}`",
                        m.raw
                    ));
                } else if m.resolved.starts_with(&crates_root) {
                    fence_base = Some(m.resolved.clone());
                }
                if is_file_shaped(&m.raw) {
                    current = Some((m.raw.clone(), m.resolved.clone(), m.resolved.is_file(), lineno));
                }
                mi += 1;
            }

            // A fragment written immediately left of the anchor is the document's own statement of
            // which file it means, and it OVERRIDES the sticky binding. Where the two agree the
            // sticky one is kept (it is the fuller path); where the fragment names something else,
            // it is resolved on its own, and if it cannot be resolved uniquely the anchor is
            // SKIPPED. Falling back to the sticky binding here is precisely what checked
            // `component_registry/mod.rs:918` against `crates/boyko_ecs/Cargo.toml`.
            match fragment_before(line, anchor.col) {
                LeftOfAnchor::Named(frag) => {
                    // The suffix must land on a path separator. Without that boundary `tags.rs`
                    // would "agree" with a sticky `component_tags.rs`, and the anchor would be
                    // checked against a file the document did not name — the very failure this
                    // override exists to remove.
                    let sticky_agrees = current
                        .as_ref()
                        .is_some_and(|(_, p, _, _)| path_ends_with_fragment(p, frag));
                    if sticky_agrees {
                        // ⚠️ The line number is refreshed even though the path is unchanged, and
                        // that is load-bearing for `DocScan::cross_line_doc`: a name written HERE
                        // is this line stating its subject, whether or not the section had already
                        // bound the same file. Without the refresh, `` `X.md:1221` `` followed by
                        // a `` `:535` `` on the SAME line reads as a cross-line inheritance.
                        // MEASURED: the census reported 399 cross-line continuations before this
                        // refresh and 373 after, and one of the 26 it dropped was that shape.
                        // (Both figures are stamped to that change and are NOT today's total —
                        // the fenced arm was blind then and contributed nothing; it contributes
                        // seven now. A past measurement is quoted as a past measurement.)
                        if let Some(c) = current.as_mut() {
                            c.3 = lineno;
                        }
                    } else {
                        // A6 merge: `fenced_resolve`, not the bare `resolve_unique_fragment`.
                        // Both sides resolved a Named fragment here and they reached different
                        // files: the line walked the section's own binding and its ancestors
                        // (`resolve_fragment`), the lane required a unique suffix match over the
                        // whole tree. `fenced_resolve` is the LANE's own union of the two, and it
                        // is what the lane already uses in the fenced arm below, so the union is
                        // this file's established combinator rather than a new rule. A fragment
                        // neither reach binds is still REFUSED into `unbindable`.
                        match fenced_resolve(frag, current.as_ref().map(|c| &c.1), fence_base.as_ref()) {
                            Some(p) => current = Some((frag.to_string(), p, true, lineno)),
                            None => {
                                unbindable.push(format!(
                                    "  {doc}:{lineno}  `{frag}:{}` names no single file in the tree",
                                    anchor.line_no
                                ));
                                continue;
                            }
                        }
                    }
                }
                LeftOfAnchor::Alias(alias) => alias_bound.push(format!(
                    "  {doc}:{lineno}  `{alias}:{}` names its target by an alias; the anchor \
                     inherits the section's binding ({})",
                    anchor.line_no,
                    current.as_ref().map_or("none", |(raw, _, _, _)| raw.as_str())
                )),
                // A continuation inherits, and the only question this gate now asks is WHERE the
                // target it inherits was named. On its own line it is the document stating its
                // subject; on an earlier one it is the sticky binding, which is the member-table
                // shape — and which is where every misbinding found so far has come from.
                LeftOfAnchor::Continuation => {
                    if let Some((sraw, spath, _, set_line)) = current.as_ref()
                        && *set_line != lineno
                    {
                        if is_md_target(spath) {
                            cross_line_doc.push(format!(
                                "  {doc}:{lineno}  `:{}` inherits `{sraw}`, named on line \
                                 {set_line}; write the document name beside the anchor",
                                anchor.line_no
                            ));
                            continue;
                        }
                        cross_line_src += 1;
                    }
                }
            }

            let Some((raw, target, exists, _)) = current.as_ref() else {
                // No anchor target since the last heading: nothing to check against.
                continue;
            };
            if !*exists {
                // The path check already reports this file once; do not pile on per member.
                continue;
            }
            let expect = paired.then(|| idents[ai].as_str());
            if let Some(class) = check_anchor(
                doc,
                lineno,
                raw,
                target,
                anchor,
                expect,
                evidence,
                &quotes_for(&para_of, &paragraphs, idx, anchor.col),
                &mut file_lines,
                &mut md_text,
                &mut anchor_violations,
                &mut over_waived,
                &mut evidence_relaxed,
                &mut doc_quote_violations,
                &mut doc_unquoted,
                &mut doc_misbound,
            ) {
                anchors += 1;
                classes[class as usize] += 1;
                if is_md_target(target) {
                    doc_to_doc += 1;
                }
            }
        }

        // Mentions to the right of the last anchor (or all of them, on a line with none).
        for m in &line_mentions[mi..] {
            mentions += 1;
            if !m.resolved.exists() {
                let bucket = if planned { &mut planned_paths } else { &mut path_violations };
                bucket.push(format!(
                    "  {doc}:{lineno}  path does not exist: `{}`",
                    m.raw
                ));
            } else if m.resolved.starts_with(&crates_root) {
                fence_base = Some(m.resolved.clone());
            }
            if is_file_shaped(&m.raw) {
                current = Some((m.raw.clone(), m.resolved.clone(), m.resolved.is_file(), lineno));
            }
        }
    }

    DocScan {
        mentions,
        anchors,
        classes,
        path_violations,
        anchor_violations,
        evidence_relaxed,
        planned_paths,
        stale_planned,
        unbindable,
        over_waived,
        doc_quote_violations,
        doc_unquoted,
        doc_to_doc,
        alias_bound,
        ignored_lines,
        silenced,
        stale_ignored,
        cross_line_doc,
        cross_line_src,
        doc_misbound,
    }
}

/// What one [`IGNORE_MARKER`] line would have reported had the marker not dropped it.
///
/// The line is re-scanned as a synthetic document, prefixed with a markdown link that restores the
/// sticky target it had in place. The link form is used rather than a bare `crates/...` mention
/// because [`ROOT_PREFIXES`] covers three prefixes and a sticky target may be under none of them; a
/// `](../…)` target resolves for any file in the tree.
///
/// # Every class the scan produces, not the three it used to report
///
/// ⚠️ **This function used to extend from `path_violations`, `anchor_violations` and
/// `doc_quote_violations` only — three of the eleven violation vectors [`DocScan`] carries — and the
/// ledger built on it was therefore a ceiling on three classes wearing the name of a ceiling on
/// silence.** MEASURED 2026-08-28: appending `` `nonexistent_probe_xyz.rs` `` with a line number to
/// the already-marked `REFLECTION-PLAN-CORE.md:2128` left the suite at exit 0 with the silenced
/// count unmoved, while the identical token on the unmarked line two below it redded at exit 101
/// with *"1 unbindable, cap 0"*. A marked line was a permanently open slot for eight of the eleven
/// classes. All eleven are extended from now.
///
/// # Two scans, because one arrangement cannot see the cross-line refusal
///
/// [`DocScan::cross_line_doc`] fires only when a continuation inherits a document bound on an
/// EARLIER line, so a re-scan that restores the binding on the anchor's own line cannot produce it
/// — `set_line == lineno` by construction. Restoring it on a preceding line instead is not a fix
/// either: every doc-to-doc anchor on the marked line would then be REFUSED rather than checked,
/// and the content violations this function exists to count would drop to zero.
///
/// So both arrangements are run, and each is read for what only it can see:
///
/// * **same-line** — the binder ahead of the text on line 1. Faithful for every class that asks
///   *"is this citation true"*, and blind to the cross-line refusal.
/// * **preceding-line** — the binder alone on line 1, the text on line 2. Read for
///   `cross_line_doc` and for nothing else.
///
/// The second arrangement is faithful rather than pessimistic, and the reason is structural: a
/// marked line is `continue`d *before* its mentions are walked, so the sticky binding a marked line
/// inherits was always set on an earlier line. A marked line that names its own target at the
/// anchor still binds through [`LeftOfAnchor::Named`] in the second scan and reports nothing, which
/// is the correct answer for it.
///
/// Stated as a limit, not implied: each re-scan sees the marked line alone, so a quotation that
/// wraps out of it is not attributable and a doc-to-doc content violation spanning the wrap is not
/// counted. The count is therefore a FLOOR on what the marker silences, which is the safe direction
/// for a ceiling that exists to stop the silence from growing.
fn silenced_by(doc: &str, line: &str, sticky: Option<&PathBuf>) -> Vec<String> {
    let bare = line.replace(IGNORE_MARKER, " ");
    let link = sticky
        .and_then(|p| p.strip_prefix(repo_root()).ok())
        .map(|rel| format!("[b](../{})", rel.to_string_lossy().replace('\\', "/")))
        .unwrap_or_default();
    let same = scan_text(doc, &format!("{link} {bare}"));
    let prior = scan_text(doc, &format!("{link}\n{bare}"));
    // Each inner scan reports against its own coordinates; the caller re-labels with the real ones.
    let strip = |s: &str, n: usize| {
        let head = format!("{doc}:{n}");
        s.trim()
            .strip_prefix(head.as_str())
            .unwrap_or(s.trim())
            .trim()
            .to_string()
    };
    let mut out = Vec::new();
    for v in [
        &same.path_violations,
        &same.anchor_violations,
        &same.planned_paths,
        &same.stale_planned,
        &same.unbindable,
        &same.over_waived,
        &same.doc_unquoted,
        &same.alias_bound,
        &same.doc_misbound,
    ] {
        out.extend(v.iter().map(|s| strip(s, 1)));
    }
    out.extend(
        same.doc_quote_violations
            .iter()
            .map(|s| strip(&s.message, 1)),
    );
    // The one class only the preceding-line arrangement can produce. Taken from `prior` and
    // nothing else is, so the two scans cannot double-count.
    out.extend(prior.cross_line_doc.iter().map(|s| strip(s, 2)));
    out
}

/// Is this anchor's target another gated document rather than a source file?
///
/// The two `check_anchor` call sites ask the same question, and it is asked at the CALL site
/// rather than inside `check_anchor` so the counter needs no thirteenth parameter on a function
/// that already carries `#[allow(clippy::too_many_arguments)]`.
fn is_md_target(target: &Path) -> bool {
    target
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("md"))
}

fn scan_all() -> BTreeMap<&'static str, DocScan> {
    GATED_DOCS.iter().map(|d| (*d, scan_doc(d))).collect()
}

/// Blanks every `~~ … ~~` strike-through span with spaces, preserving line structure.
///
/// A span may cross lines (the register strikes whole sentences). The three-tilde code-fence
/// marker `~~~` at a line start is NOT a strike and toggles nothing; a fenced block's contents are
/// left alone entirely, because a `~~` inside a code fence is code, not markdown.
fn blank_struck(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut struck = false;
    let mut in_fence = false;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            out.push_str(line);
            continue;
        }
        if in_fence {
            out.push_str(line);
            continue;
        }
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'~' && i + 1 < bytes.len() && bytes[i + 1] == b'~' {
                struck = !struck;
                out.push_str("  ");
                i += 2;
                continue;
            }
            let ch = line[i..].chars().next().expect("invariant: i is on a char boundary");
            if struck && ch != '\n' && ch != '\r' {
                out.push(' ');
            } else {
                out.push(ch);
            }
            i += ch.len_utf8();
        }
    }
    out
}

#[test]
fn a_struck_anchor_is_not_a_claim_and_an_unstruck_one_still_is() {
    // Red-first: with `blank_struck` replaced by the identity, the first two assertions fail.
    let same_line = "see ~~`crates/x/src/a.rs:370` (`push_task`)~~ later";
    let blanked = blank_struck(same_line);
    assert!(!blanked.contains("a.rs:370"), "a struck same-line anchor must be blanked: {blanked:?}");
    assert_eq!(blanked.len(), same_line.len(), "blanking must preserve byte length");

    let multi_line = "one ~~`crates/x/src/a.rs:370`\nstill struck `b.rs:9`~~ live `c.rs:5`";
    let blanked = blank_struck(multi_line);
    assert!(!blanked.contains("a.rs:370") && !blanked.contains("b.rs:9"), "{blanked:?}");
    assert!(blanked.contains("c.rs:5"), "text after the closing strike is live: {blanked:?}");
    assert_eq!(blanked.lines().count(), multi_line.lines().count(), "line structure preserved");

    let fenced = "~~~\n~~not a strike inside a fence `d.rs:1`~~\n~~~\n~~struck `e.rs:2`~~";
    let blanked = blank_struck(fenced);
    assert!(blanked.contains("d.rs:1"), "a fence's contents are untouched: {blanked:?}");
    assert!(!blanked.contains("e.rs:2"), "a strike after the fence still blanks: {blanked:?}");
}

#[test]
fn internal_docs_cite_paths_that_exist() {
    let scans = scan_all();
    let mut report = String::new();

    for (doc, scan) in &scans {
        // Printed so the module doc's counts stay re-derivable from the gate itself
        // (`cargo test ... -- --nocapture`) rather than being folklore copied between commits.
        println!(
            "docs/{doc}: {} path mention(s) checked, {} dead",
            scan.mentions,
            scan.path_violations.len()
        );
        // Non-emptiness is asserted per document: a broken pattern, a renamed file or a document
        // gutted to a stub must fail here rather than pass vacuously behind a healthy neighbour.
        assert!(
            scan.mentions > 0,
            "docs/{doc}: extracted ZERO path mentions. Either the document no longer cites the \
             source tree, or the extraction in this test is broken. A gate that passes over an \
             empty set is not a gate."
        );
        if !scan.path_violations.is_empty() {
            report.push_str(&format!(
                "\ndocs/{doc}: {} dead path(s) of {} mention(s)\n",
                scan.path_violations.len(),
                scan.mentions
            ));
            report.push_str(&scan.path_violations.join("\n"));
            report.push('\n');
        }
    }

    assert!(
        report.is_empty(),
        "gated internal docs cite paths that do not exist.\n\
         The navigation documents are the mandated first point of contact (CLAUDE.md) and the \
         register documents are the evidence a ruled ballot stands on; a dead path sends the \
         reader nowhere in either.\n{report}"
    );
}

#[test]
fn internal_docs_line_anchors_land_on_definitions() {
    let scans = scan_all();
    let mut report = String::new();

    for (doc, scan) in &scans {
        println!(
            "docs/{doc}: {} anchor(s) checked, {} stale",
            scan.anchors,
            scan.anchor_violations.len()
        );
        assert!(
            scan.anchors > 0,
            "docs/{doc}: extracted ZERO line anchors. Either the document stopped citing line \
             numbers, or the extraction in this test is broken. A gate that passes over an empty \
             set is not a gate."
        );

        // The decomposition is PRINTED, not remembered. Four revisions of this gate's own
        // documentation quoted anchor counts that no run reproduced, because the numbers were
        // transcribed once and then carried. Deriving them here means the only figure anyone can
        // cite is one the shipped scanner just produced.
        let waived = scan.count(AnchorClass::Waived);
        let unpaired = scan.count(AnchorClass::Unpaired);
        let undeclared = scan.count(AnchorClass::PairedUndeclared);
        let identity = scan.count(AnchorClass::Identity);
        let prose = scan.count(AnchorClass::Prose);
        assert_eq!(
            waived + unpaired + undeclared + identity + prose,
            scan.anchors,
            "docs/{doc}: the class histogram does not account for every counted anchor — the \
             decomposition is the only thing that makes the coverage claim checkable, so a \
             mismatch here means the claim is unbacked."
        );
        println!(
            "docs/{doc}: {} anchors = {identity} identity-asserted + {unpaired} shape-only \
             (unpaired) + {undeclared} shape-only (symbol not declared in the cited file) + \
             {waived} waived (`~`: neither shape nor identity, only the in-file bounds check) + \
{prose} prose (`.md` target: bounds only, markdown declares nothing)",
            scan.anchors
        );
        if scan.evidence_relaxed > 0 {
            println!(
                "docs/{doc}: {} of those sit on a non-definition line and were accepted as EVIDENCE citations (this document is in EVIDENCE_DOCS: the shape test is not run on it; the bounds, range and identity checks still are)",
                scan.evidence_relaxed
            );
        }
        if !scan.anchor_violations.is_empty() {
            report.push_str(&format!(
                "\ndocs/{doc}: {} stale anchor(s) of {} checked\n",
                scan.anchor_violations.len(),
                scan.anchors
            ));
            report.push_str(&scan.anchor_violations.join("\n"));
            report.push('\n');
        }
    }

    assert!(
        report.is_empty(),
        "gated internal docs cite line numbers that no longer hold what they name.\n\
         Re-derive each anchor from the current source.\n\
         THE NARROWINGS IN FORCE, so a reader can tell a real finding from a genre mismatch \
         (GB-8: narrow the predicate and PRINT the narrowing): a document in EVIDENCE_DOCS is not \
         shape-tested at all, so an identity failure there means the citation landed on a \
         DECLARATION that declares something else — a real finding, not a doc comment being cited \
         as evidence. On a document outside EVIDENCE_DOCS a deliberate non-definition anchor is \
         marked `:N~` / `(N~)` rather than loosening the check for everyone; do NOT reach for that \
         form to silence a whole document's genre — that is what EVIDENCE_DOCS is for, and it is \
         one greppable declaration instead of fifty waivers.\n{report}"
    );
}

/// Sensitivity control for the range tail — the only part of this gate that had **no** control and
/// was, until Rev 12, unreachable on **the majority of** the corpus it was written for.
///
/// `range_end` feeds two assertions in `check_anchor`: a range whose end precedes its start, and a
/// range whose end is past EOF. Both are dead whenever the parser fails to capture `M`. The parser
/// tested `-` before `~`, so it recognised `:N-M~` and stopped at the `~` in `:N~-M`. Measured on
/// the meshlet plan: **34 occurrences waiver-first, 23 plain** — the gate ran green over that
/// document with the tail check unable to fire on the 34, while the 23 reached it normally.
///
/// ⚠️ Rev 12 wrote "**every** range citation it has" here and in three sibling texts. That was a
/// volunteered claim inside a repair and it was false; the corrected split is above.
///
/// This control asserts the capture directly, in both spellings, because a green corpus run cannot
/// distinguish "no incoherent range exists" from "no range was parsed".
#[test]
fn a_range_tail_is_captured_with_the_waiver_written_on_either_side() {
    let cases: [(&str, bool); 3] = [
        // The spelling the plan actually uses, and the one that was silently dropped.
        ("see `crates/boyko_ecs/src/lib.rs:94~-96` here", true),
        // The spelling the parser already handled, kept so a fix for one cannot break the other.
        ("see `crates/boyko_ecs/src/lib.rs:94-96~` here", true),
        // Unwaived, to pin that accepting the waiver did not make it mandatory.
        ("see `crates/boyko_ecs/src/lib.rs:94-96` here", false),
    ];
    for (text, waived) in cases {
        let (_, anchors) = scan_line(text, &docs_dir());
        assert_eq!(anchors.len(), 1, "expected exactly one anchor in {text:?}");
        assert_eq!(anchors[0].line_no, 94, "start line, in {text:?}");
        assert_eq!(
            anchors[0].range_end,
            Some(96),
            "the range tail must be captured in {text:?} — without it the end-before-start and \
             end-past-EOF checks in check_anchor cannot fire for this citation"
        );
        assert_eq!(anchors[0].shape_waived, waived, "waiver state, in {text:?}");
    }
}

/// The companion control: the capture above must be exercised by the **live** corpus, not merely by
/// a synthetic string. A parser regression that dropped the tail again would leave the test above
/// green only if it were also edited, but would silently empty this count.
#[test]
fn the_gated_docs_actually_exercise_the_range_tail() {
    // ⚠️ Counted PER DOCUMENT, not aggregated, because an aggregate floor lets the document the
    // check was written for fall to zero while the others carry the sum. Rev 13 justified this
    // change with "a live total of 33 — a margin of three"; that figure was never re-derived and
    // is wrong (the plan alone parses 57, which this very function prints three lines below).
    // The argument for splitting the count does not depend on the margin and stands without it.
    let mut per_doc: Vec<(&str, usize)> = Vec::new();
    for doc in GATED_DOCS {
        let text = std::fs::read_to_string(docs_dir().join(doc))
            .unwrap_or_else(|e| panic!("read docs/{doc}: {e}"));
        let doc_dir = doc_dir_of(doc);
        let n = text
            .lines()
            .map(|l| {
                scan_line(l, &doc_dir)
                    .1
                    .iter()
                    .filter(|a| a.range_end.is_some())
                    .count()
            })
            .sum();
        println!("docs/{doc}: {n} range citation(s) parsed a tail");
        per_doc.push((doc, n));
    }
    let with_tail: usize = per_doc.iter().map(|(_, n)| n).sum();

    // ⚠️ A FLOOR ON A SUM CANNOT BE SENSITIVE TO LOSING ONE SPELLING, and Rev 13's `plan >= 20`
    // was inert for exactly that reason: the regression its own message named — the parser testing
    // `-` before `~` again — leaves the plan's 23 plain citations parsing tails, and 23 >= 20
    // passes; the mirror leaves 34 and also passes. Only total parser death reached it. What IS
    // sensitive is asserting that BOTH SPELLINGS are exercised, which is the property the repair
    // actually established.
    let plan_text = std::fs::read_to_string(docs_dir().join("MESHLET-VIRTUAL-GEOMETRY-PLAN.md"))
        .expect("invariant: the meshlet plan is in GATED_DOCS");
    let (mut waived_ranges, mut plain_ranges) = (0usize, 0usize);
    for line in plan_text.lines() {
        for a in scan_line(line, &docs_dir()).1.iter().filter(|a| a.range_end.is_some()) {
            if a.shape_waived {
                waived_ranges += 1;
            } else {
                plain_ranges += 1;
            }
        }
    }
    println!(
        "docs/MESHLET-VIRTUAL-GEOMETRY-PLAN.md: {waived_ranges} waived + {plain_ranges} plain range \
         citation(s) parsed a tail"
    );
    assert!(
        waived_ranges > 0,
        "no WAIVER-BEARING range citation in the meshlet plan parsed a tail. That is the Rev 11 \
         defect recurring verbatim: the parser tested `-` before `~`, so every `:N~-M` lost its \
         tail and both end-of-range assertions became unreachable on the majority of that \
         document. A sum-based floor cannot see this — the plain citations keep the total up."
    );
    assert!(
        plain_ranges > 0,
        "no PLAIN `:N-M` range citation in the meshlet plan parsed a tail — the mirror regression, \
         in which accepting the waiver-first spelling broke the spelling that always worked. \
         Stated separately from the waived count because a check that cannot distinguish the two \
         directions is the check that missed the first one."
    );
    // A floor, not a pin: the exact number moves with ordinary editing, and pinning it would make
    // every citation edit a test edit. Zero is the only value that means the check is dead.
    assert!(
        with_tail >= 30,
        "only {with_tail} range citations across GATED_DOCS parsed a range tail. The end-of-range \
         checks in check_anchor are reached only through `range_end`, so a collapse here means \
         they have stopped running — which is how they spent the whole of Rev 11 dead on the 34 \
         waiver-first range occurrences in the meshlet plan (the 23 plain ones did reach them)."
    );
}

/// Controls for the binding repair, the two extraction rejections, and the shape fix.
///
/// Each of these mechanisms was added or corrected because a WRONG result was measured on the live
/// corpus, and each is asserted here directly rather than through a corpus run — a green corpus
/// cannot distinguish "the rule works" from "the rule never fired". That distinction is the single
/// most-repeated defect in this campaign, and it is what the `#[test]` below exists to deny.
#[test]
fn the_binding_and_extraction_rules_are_asserted_directly() {
    // --- Fragment resolution: unique resolves, ambiguous and absent do NOT guess. ---
    let unique = resolve_unique_fragment("component_registry/tags.rs")
        .expect("invariant: exactly one component_registry/tags.rs exists in this tree");
    assert!(
        unique.ends_with("tags.rs"),
        "a unique fragment must resolve to the file it names, got {}",
        unique.display()
    );
    assert_eq!(
        resolve_unique_fragment("mod.rs"),
        None,
        "`mod.rs` names dozens of files here. Resolving it would pick one arbitrarily, which is \
         the misbinding this function exists to prevent — zero and many must both refuse."
    );
    assert_eq!(
        resolve_unique_fragment("no_such_file_in_this_tree_xyz.rs"),
        None,
        "an absent fragment must refuse rather than resolve"
    );
    // The path-segment boundary: `tags.rs` must not match `component_tags.rs`.
    assert!(
        !path_ends_with_fragment(Path::new("crates/x/src/component_tags.rs"), "tags.rs"),
        "a fragment must land on a path separator; a bare suffix test binds one file's citations \
         to another file whose name merely ends the same way"
    );
    assert!(path_ends_with_fragment(
        Path::new("crates/x/src/component_registry/tags.rs"),
        "component_registry/tags.rs"
    ));

    // --- `ROOT_PREFIXES`: a bare `.github/...` mention is seen at all. ---
    let (mentions, anchors) = scan_line("`--exclude boyko_demo` at `.github/workflows/ci.yml:62`", &docs_dir());
    assert_eq!(
        mentions.len(),
        1,
        "a bare `.github/...` mention must be extracted. While it was not, the anchor behind it \
         fell through to the section's last `crates/...` path and was checked against that file."
    );
    assert!(mentions[0].resolved.is_file(), "and it must resolve on disk");
    assert_eq!(anchors.len(), 1);
    assert_eq!(anchors[0].line_no, 62);

    // --- A glob is not a path claim; the literal beside it still is. ---
    assert_eq!(
        scan_line("its record is `docs/PHASE-*-RESULTS.md`.", &docs_dir()).0.len(),
        0,
        "a glob names a FAMILY. Read as a path it truncates at the `*` to `docs/PHASE-`, then \
         `docs/PHASE` — a file that was never claimed to exist, reported dead in two documents."
    );
    assert_eq!(
        scan_line("see `docs/FEATURE_MAP.md` for the map", &docs_dir()).0.len(),
        1,
        "rejecting globs must not reject literal paths under the same prefix"
    );

    // --- A ratio is not an anchor; a `(a) / (b)` list still is. ---
    assert_eq!(
        scan_line("the two targets sum to exactly 2.000 (3840/1920 = 2.000)", &docs_dir()).1.len(),
        0,
        "`(3840/1920` was parsed as an anchor on line 3840. The `/` alternative is for a member \
         list separator, never for a slash sitting directly between two digits."
    );
    let list = scan_line("`spawn_one` (582) / `spawn_batch` (611)", &docs_dir()).1;
    assert_eq!(
        list.len(),
        2,
        "rejecting ratios must not break the `(a) / (b)` member list the `/` was added for"
    );
    assert_eq!((list[0].line_no, list[1].line_no), (582, 611));

    // --- A prose enumerator is not a citation; a trailing member ref still is. ---
    assert_eq!(
        scan_line("**Landed notes (2026-08-21, recorded at execution).** (1) Gate 5 requires the", &docs_dir())
            .1
            .len(),
        0,
        "`(1)` opening an enumerated clause is not an anchor on line 1. Reading it as one produced \
         four `stale` reds in REFLECTION-PLAN-GATES.md, and the first repair pass answered them by \
         writing `(1~)` `(2~)` `(3~)` INTO the prose — a scanner false positive laundered into the \
         document as a deliberate citation."
    );
    assert_eq!(
        scan_line("the blindness as (1) and the reason (2) is carried", &docs_dir()).1.len(),
        0,
        "a numbered reference back to an earlier item is prose, not a citation"
    );
    assert_eq!(
        scan_line("G0's census (3 tests), G1 (7), G2 (2), G3 (2, the", &docs_dir()).1.len(),
        0,
        "counts in parentheses are quantities; `(2,` even satisfies the comma continuation"
    );
    let member = scan_line("| everything | `clear()` (1026) |", &docs_dir()).1;
    assert_eq!(
        member.len(),
        1,
        "the member-table form must survive: it is the densest citation shape in these documents"
    );
    assert_eq!(member[0].line_no, 1026);

    // --- Definition shape: generic items count; a `pub(crate)` FIELD still does not. ---
    assert!(
        looks_like_definition("impl<S: States> Resource for State<S> {", "rs"),
        "a generic impl is a definition. Requiring `impl ` with a trailing space excluded every \
         one of them, and reported `state.rs:43` stale to three separate paragraphs."
    );
    assert!(
        looks_like_definition("pub use scope::Scope;", "rs"),
        "`pub ` keeps its one-sided rule; demanding a non-identifier byte after the space rejects \
         every `pub use` and every `pub field: T`, which are long-standing passes"
    );
    assert!(
        !looks_like_definition("pub(crate) enable_store: EnableStore,", "rs"),
        "a struct field is NOT a definition — it is the canonical `~` waiver class in these \
         documents, and widening `pub` to match `pub(` reclassifies it in two incumbent documents"
    );
    assert!(
        !looks_like_definition("    implementation_note();", "rs"),
        "the two-sided boundary must still reject a keyword that heads a longer identifier"
    );
}

/// Paths a plan names as deliverables it has not built — reported and pinned.
///
/// This is the ledger of what the reflection campaign still owes, derived from the plans rather
/// than tracked beside them. It is pinned so the marker cannot be used to quiet an ordinary dead
/// path: raising a ceiling here is a claim that the plan grew a NEW unbuilt deliverable, which is a
/// thing a reviewer can check.
///
/// ⚠️ The number is expected to go **down** as the campaign lands its rungs. A ceiling is the wrong
/// shape for that — it goes quiet exactly when the work finishes — so this asserts EQUALITY. When a
/// deliverable lands, its marker comes off and this number is decremented in the same commit.
#[test]
fn planned_paths_are_reported_and_pinned() {
    /// Exact count of `<!-- doc-path-planned -->` mentions per document.
    const PLANNED_EXACT: &[(&str, usize)] = &[
        ("ARCHITECTURE.md", 0),
        ("FEATURE_MAP.md", 0),
        ("MESHLET-VIRTUAL-GEOMETRY-PLAN.md", 0),
        ("REFLECTION-ANALYSIS.md", 0),
        // 5 → 1 on 2026-08-27: rung B0 LANDED, building four of the five — `fixtures/mod.rs`,
        // `fixtures/ids.rs`, `boundary_roundtrip.rs`, `boundary_id_reorder.rs` — and their markers
        // came off in this same change. Only B4's `format_divergence_ledger.rs` is still unbuilt.
        // (4 → 5 earlier the same day, BOUNDARY D22: the B0 audit moved `CAPTURED_POD3_ID` off rung
        // B5's Lands and onto B0's, writing a repo-relative path where B5 had named
        // `acceptance_ids.rs` with none — which is what made an always-unbuilt deliverable VISIBLE
        // to this pin in the first place.)
        ("REFLECTION-PLAN-BOUNDARY.md", 1),
        ("REFLECTION-PLAN-CORE.md", 0),
        // 0 → 3 on 2026-08-27, EG2 AUDIT (that rung's D26): the rung named FOUR public kernel
        // items and NOT ONE FILE, so its whole test surface was invisible to this pin — it could
        // not go 1 → 0, could not go stale, and nothing would have redded if the tests were never
        // written. The three are `ecs_master/seam_by_id.rs`, `boyko_ecs/tests/seam_by_id.rs` and
        // the `seam_pass` corpus that EG2's gate 11 migrates the flipped fixtures into. Back to 0
        // when EG2 lands, markers off in the same commit (D25: the marker half is not self-checking).
        // 3 → 1 on 2026-08-27: EG2's KERNEL half LANDED two of the three —
        // `ecs_master/seam_by_id.rs` and `boyko_ecs/tests/seam_by_id.rs` — and their markers came
        // off in this same change. The third, `crates/boyko_reflect/tests/seam_pass/`, belongs to
        // EG2's gate 11 (the census migration), which lands separately and takes this to 0. The
        // split is not a preference: creating the two files WITHOUT removing their markers reds
        // `planned_paths_are_reported_and_pinned` (measured, exit 101, "2 stale marker(s)"), so the
        // marker half of this pin is owned by whoever creates the file, not by whoever finishes
        // the rung.
        // 1 → 0 on 2026-08-27: EG2's GATE 11 landed the third and last —
        // `crates/boyko_reflect/tests/seam_pass/` — by moving the four flipped `trybuild` fixtures
        // into it and deleting their `.stderr`; its marker came off in this same change. EG2 is
        // now whole and this document owes no unbuilt deliverable.
        ("REFLECTION-PLAN-ECS.md", 0), // was 1 → 0 on 2026-08-26: EG1 built `ecs_alloc.rs`, the last marker this document carried. (2 → 1 the same day: EG0 built `seam_census.rs`.) Kept on ONE line: a bare `:1906-1941` fragment below cites this file and shifts silently.
        // 4 → 3 on 2026-08-26: CORE C9 built G5's `reflect_compile_fail.rs`, so its
    // `doc-path-planned` marker came off in the same edit as this decrement.
    ("REFLECTION-PLAN-GATES.md", 3),
        ("SYSTEMS.md", 0),
        // The register corpus, which `GATED_DOCS` admitted on 2026-09-10 but which this census
        // never saw: this map is the reflection lane's, authored against the nine documents that
        // existed on its own branch. The six below are therefore a FIRST measurement, taken on the
        // merged tree - they are what the run reports, not a target. See `OVER_WAIVED_MAX`, the one
        // per-document map both sides had, for the same note in the line's own words.
        ("OPEN-QUESTIONS.md", 0),
        ("AETHER-GAIA-REVISION-2026-08-29.md", 0),
        ("gaia/CAMPAIGN.md", 0),
        ("gaia/DECISIONS.md", 0),
        ("gaia/LANGUAGE.md", 0),
        ("gaia/PENDING-SYNTAX-PLAN.md", 0),
    ];

    let scans = scan_all();
    let mut report = String::new();
    let mut stale = String::new();

    for (doc, scan) in &scans {
        let n = scan.planned_paths.len();
        let want = PLANNED_EXACT
            .iter()
            .find(|(d, _)| d == doc)
            .map(|(_, c)| *c)
            .unwrap_or_else(|| panic!("docs/{doc} has no entry in PLANNED_EXACT"));
        println!("docs/{doc}: {n} planned-but-unbuilt path(s) (pinned at {want})");
        if n != want {
            report.push_str(&format!(
                "docs/{doc}: {n} planned-but-unbuilt path(s), pinned at {want}\n{}\n",
                scan.planned_paths.join("\n")
            ));
        }

        // The OTHER half of the same contract: a marker that waives nothing. Printed per
        // document like the count above, so its zero is a measured zero and not an absent line.
        let s = scan.stale_planned.len();
        println!("docs/{doc}: {s} stale `{PLANNED_MARKER}` marker(s)");
        if s != 0 {
            stale.push_str(&format!("docs/{doc}:\n{}\n", scan.stale_planned.join("\n")));
        }
    }

    assert!(
        stale.is_empty(),
        "a `{PLANNED_MARKER}` marker waives nothing -- every path on its line is already on \
         disk. DELETE THE MARKER.\n\
         It is not inert: it suppresses the dead-path check on that line for every future \
         edit, so the next path written there goes unchecked.\n\
         And it is the half of a `Lands`-gate pair that was previously unobservable -- the \
         pin below decrements because the FILE appeared, not because the marker came off, \
         so the marker could stay and the rung still report green.\n{stale}"
    );

    assert!(
        report.is_empty(),
        "the plans' unbuilt-deliverable ledger moved.\n\
         DOWN means a deliverable landed: remove its `<!-- doc-path-planned -->` marker so the path \
         is checked like any other, and decrement the pin here in the same commit.\n\
         UP means a plan named a new artifact it has not built — or that an ordinary path rotted \
         and someone reached for the marker instead of re-deriving it. Only the first is a reason \
         to raise the pin.\n{report}"
    );
}

/// Anchors whose own file fragment resolves to no single file — reported and pinned.
///
/// These are the anchors the gate **cannot** check. Before [`resolve_unique_fragment`] existed they
/// were not visible as a gap at all: they were checked, loudly and wrongly, against whatever file
/// the section last named. Skipping them is correct and misbinding them was not, but a skip is
/// still lost coverage, and lost coverage that nobody counts is how a census ends up green over
/// nothing — the defect this whole file exists to prevent.
///
/// Two shapes reach here, and they want opposite responses:
///
/// * **Ambiguous** — `mod.rs`, `lib.rs`, `component.rs` name dozens of files in this tree. Only the
///   document can say which, by writing more of the path. Fixing one is a doc edit.
/// * **Absent** — the fragment names a file that no longer exists anywhere. That is real rot, and
///   it hides here instead of in the path check because a fragment is not a path mention.
///
/// The ceiling is per document so one document's improvement cannot pay for another's regression.
#[test]
fn unbindable_fragments_are_reported_and_pinned() {
    /// Per-document ceiling on anchors skipped for an unresolvable file fragment.
    const UNBINDABLE_MAX: &[(&str, usize)] = &[
        ("ARCHITECTURE.md", 0),
        ("FEATURE_MAP.md", 0),
        ("MESHLET-VIRTUAL-GEOMETRY-PLAN.md", 0),
        ("REFLECTION-ANALYSIS.md", 0),
        ("REFLECTION-PLAN-BOUNDARY.md", 0),
        ("REFLECTION-PLAN-CORE.md", 0),
        ("REFLECTION-PLAN-ECS.md", 0),
        ("REFLECTION-PLAN-GATES.md", 0),
        ("SYSTEMS.md", 0),
        // The register corpus, which `GATED_DOCS` admitted on 2026-09-10 but which this census
        // never saw: this map is the reflection lane's, authored against the nine documents that
        // existed on its own branch. The six below are therefore a FIRST measurement, taken on the
        // merged tree - they are what the run reports, not a target. See `OVER_WAIVED_MAX`, the one
        // per-document map both sides had, for the same note in the line's own words.
        //
        // ⚠️ **32 -> 36 in the A7 merge, and all four are the UI campaign's.** The union inserted
        // A7's register sections, which no census in this file had read, and every member was
        // attributed to the side that inserted its citing line: the line's 32 are unchanged and
        // A7's sections brought 16. Twelve of those were made unique and re-derived by exact text
        // from the commit that wrote them. The four left are a grep record of
        // `host_upload_frame`'s callers whose two `upload.rs` numbers did not hold even at their
        // authoring commit, the gate's own misreport quoted verbatim as a finding (`components.rs`
        // 1117), and a `pack.rs` 205 whose authoring text is not the rule its sentence names.
        // Naming a file for those four would bind a number nobody can vouch for.
        ("OPEN-QUESTIONS.md", 36),
        ("AETHER-GAIA-REVISION-2026-08-29.md", 0),
        ("gaia/CAMPAIGN.md", 0),
        ("gaia/DECISIONS.md", 2),
        ("gaia/LANGUAGE.md", 0),
        ("gaia/PENDING-SYNTAX-PLAN.md", 0),
    ];

    let scans = scan_all();
    let mut report = String::new();
    let mut failed = false;

    for (doc, scan) in &scans {
        let n = scan.unbindable.len();
        let cap = UNBINDABLE_MAX
            .iter()
            .find(|(d, _)| d == doc)
            .map(|(_, c)| *c)
            .unwrap_or_else(|| panic!("docs/{doc} has no entry in UNBINDABLE_MAX"));
        println!("docs/{doc}: {n} anchor(s) skipped for an unresolvable fragment (cap {cap}); this ledger's live population is EMPTY in every document, so its mechanism is discriminated by `a_fenced_note_naming_no_single_file_is_refused_rather_than_rebound` and not by this row");
        if n > cap {
            failed = true;
            report.push_str(&format!(
                "docs/{doc}: {n} unbindable, cap {cap}\n{}\n",
                scan.unbindable.join("\n")
            ));
        }
    }

    assert!(
        !failed,
        "more anchors are skipped for an unresolvable file fragment than the pinned ceiling.\n\
         An anchor whose fragment names no single file is NOT checked at all. Write enough of the \
         path to make it unique (`component_registry/mod.rs`, not `mod.rs`) rather than raising \
         this ceiling — raising it buys a green by shrinking the gate.\n{report}"
    );
}

/// Waivers sitting on definition-shaped lines — reported and pinned, because a waiver that buys
/// nothing is a silently weakened assertion.
///
/// ⚠️ The meshlet plan entered `GATED_DOCS` with 102 of its ~200 anchors carrying `~`, applied in
/// bulk by a textual applicator, and an adversarial review held that "at least four" of them sit on
/// citations naming a definition — i.e. that the waiver gave up a check that would have passed.
/// This finds all of them instead of four. It found **six** in the plan and one in SYSTEMS.md.
///
/// **Then measurement refuted the diagnosis for all six.** Dropping their `~` does not turn them
/// green: every one reports ``does not define X`` where X is the symbol of the *neighbouring*
/// anchor. The cause is the pairing rule two bullets up — pairing is positional, and this document
/// writes the symbol *after* its citation (``(`:279`) producing a `Coverage` (`:211`) of
/// `CoveredPixel` (`:193`)``), so anchor *i* is attributed symbol *i* while the prose intends
/// symbol *i+1*. The anchors are correct; the attribution is off by one. The waiver was suppressing
/// a false positive, which is exactly what the plan's §12 says the waivers are for. SYSTEMS.md's
/// one was a genuine over-waiver and is now checked.
///
/// So this stays a **pinned report, not a violation**, and the ceiling is the count that survived
/// that test. A definition-shaped line can still be the wrong definition, and — as measured here —
/// a waiver on one can still be load-bearing. What must not happen is the number growing without
/// anyone noticing, which is precisely how 102 waivers arrived in a single commit.
#[test]
fn waivers_that_were_not_needed_are_reported_and_pinned() {
    /// Per-document ceiling on waivers sitting on definition-shaped lines.
    const OVER_WAIVED_MAX: &[(&str, usize)] = &[
        ("ARCHITECTURE.md", 0),
        ("FEATURE_MAP.md", 0),
        // 6 → 7, 2026-08-20, and the growth was checked rather than absorbed. All seven waivers
        // were dropped and the gate re-run: it reported seven STALE anchors, every one the
        // off-by-one attribution this test's doc describes — `:279` paired with `Coverage` while
        // the line reads `pub fn rasterize(`, `:211` with `CoveredPixel` while the line reads
        // `pub struct Coverage {`, and so on down all seven. So all seven suppress false
        // positives and none is a genuine over-waiver. Restored, ceiling raised.
        //
        // ⚠️ The panic message below tells the reader to drop the `~` and lower the ceiling.
        // That advice is right for a genuine over-waiver and WRONG for this shape, where it
        // manufactures stale anchors out of correct ones — measured, not supposed.
        // ⚠️ 2026-09-10: this cap was momentarily 9, and the measurement said not to raise it.
        // Binding an anchor to the file it NAMES (see `scan_doc`) made two more waivers visible —
        // `loaders/obj.rs:60~` (:486) and `sv0_scene/mod.rs:149~` (:1006), previously charged to
        // another file entirely — and the drop-and-rerun this message prescribes was run on both:
        // the plan reports **0 stale** without them and the count falls back to 7. They were
        // genuine over-waivers, buying nothing, so they are REMOVED from the document rather than
        // absorbed by a higher ceiling.
        ("MESHLET-VIRTUAL-GEOMETRY-PLAN.md", 7),
        // The zero is a real ceiling rather than an unmeasured default — the arming run printed
        // 0 over-waivers for each of the five — but NOT for the reason first written here.
        //
        // ~~"The reflection documents write no `~` waivers at all — they cite definitions, not
        // evidence lines."~~ MEASURED 2026-08-21, immediately after the arming: the five carry
        // **242** waivers between them, 183 of them in CORE alone. They cite evidence lines
        // constantly.
        //
        // What is zero is the OVER-waiver count, which is a different quantity: no waiver in
        // these five sits on a definition-shaped line. That is the property this ceiling pins,
        // and it survives the correction intact.
        //
        // The false sentence is struck rather than deleted because of the direction it failed
        // in: it invited a future reader to trust a number whose stated reason had stopped
        // being true, which is the one way a sound gate still misleads.
        ("REFLECTION-ANALYSIS.md", 0),
        ("REFLECTION-PLAN-BOUNDARY.md", 0),
        ("REFLECTION-PLAN-CORE.md", 0),
        ("REFLECTION-PLAN-ECS.md", 0),
        ("REFLECTION-PLAN-GATES.md", 0),
        ("SYSTEMS.md", 0),
        // The register corpus, measured on entry 2026-09-10 (see `GATED_DOCS`). These documents
        // were never authored against this gate, so their ceilings are the counts the first run
        // reported, not aspirations — lower one only with a measurement behind it.
        ("OPEN-QUESTIONS.md", 0),
        ("AETHER-GAIA-REVISION-2026-08-29.md", 0),
        ("gaia/CAMPAIGN.md", 0),
        ("gaia/DECISIONS.md", 0),
        ("gaia/LANGUAGE.md", 0),
        ("gaia/PENDING-SYNTAX-PLAN.md", 0),
    ];

    let scans = scan_all();
    let mut report = String::new();
    let mut failed = false;

    for (doc, scan) in &scans {
        let n = scan.over_waived.len();
        let cap = OVER_WAIVED_MAX
            .iter()
            .find(|(d, _)| d == doc)
            .map(|(_, c)| *c)
            .unwrap_or_else(|| panic!("docs/{doc} has no entry in OVER_WAIVED_MAX"));
        println!("docs/{doc}: {n} waived anchor(s) on definition-shaped lines (cap {cap})");
        if n > cap {
            failed = true;
            report.push_str(&format!(
                "docs/{doc}: {n} over-waivers, cap {cap}\n{}\n",
                scan.over_waived.join("\n")
            ));
        }
    }

    assert!(
        !failed,
        "more anchors are waived-yet-definition-shaped than the pinned ceiling.\n\
         A `~` waives BOTH the shape and the identity assertion, so putting one on a line that \
         would have passed silently weakens the gate.\n\
         CHECK BEFORE YOU DROP: definition-shaped does NOT mean the identity check would pass. \
         Where the prose runs SYMBOL then anchor — ``Coverage` (`:211`)` — the pairing attributes \
         anchor i to symbol i+1, so the waiver is suppressing a false positive and dropping it \
         MANUFACTURES a stale anchor out of a correct one. Drop one `~`, re-run \
         `internal_docs_line_anchors_land_on_definitions`, and read what it says: a genuine \
         over-waiver goes green, an off-by-one victim reports `does not define X` against the \
         line the PREVIOUS symbol defines. Lower the ceiling only for the ones that went green; \
         raise it, with the measurement, for the ones that did not.\n{report}"
    );
}

/// A `X.md:N` citation must land on the text it quotes — not merely inside the file.
///
/// See the banner above [`normalize_md`] for the measurement that motivated this. In short: this
/// census bounds-checked doc-to-doc anchors and nothing else, so `REFLECTION-PLAN-ECS.md:1851`
/// stood at three sites in `REFLECTION-PLAN-CORE.md` for a sentence that lives at `:1853`, while
/// the blank-line anchor it replaced had reddened the census. The instrument was quieter about the
/// worse anchor.
#[test]
fn doc_to_doc_anchors_carry_the_text_they_quote() {
    /// Stale doc-to-doc citations this check FOUND and that the finding round did NOT repair.
    ///
    /// `(citing doc, citing line, cited doc, the line cited, the line the words are ACTUALLY on)`.
    ///
    /// ⚠️ **A NAMED LIST, not a ceiling, and the difference is the whole point.** Every other
    /// escape hatch in this file pins a COUNT, and a count lets one repaired anchor silently pay
    /// for one freshly broken one. A row is exempt individually: an unlisted violation fails, and
    /// so does a LISTED one that has gone green — repairing an anchor means deleting its row here,
    /// in the same edit, the same discipline `DocScan::stale_planned` enforces on the planned-path
    /// marker.
    ///
    /// **EMPTY as of EG2-R round 4: all five were repaired by the record act, and this list
    /// enforced its own emptying — every one of the five reported "listed in KNOWN_STALE but no
    /// longer reports" until its row was deleted.** For the record, because four of the five were
    /// repaired to the target this list named and the fifth was NOT:
    ///
    /// * `BOUNDARY:665` → `CORE:3811` became `:3810-3811`. The quotation *"fixture is local
    ///   (`struct StrFixture { s: String }`)"* spans two lines and BEGINS on `:3810`.
    /// * `BOUNDARY:666` → `ECS:1502` became `:1618`.
    /// * `BOUNDARY:1191` → `ECS:2085` became `:2201`.
    /// * `CORE:1024` → `BOUNDARY:1386-1389` became `:1459-1462` (both ends read: `:1459` is the
    ///   numbered item's first line, `:1462` its last).
    /// * ⚠️ `CORE:2544` → **the listed target `:1221` was WRONG, and following it would have
    ///   created a fresh instance of the very defect this check exists to catch.** `:1220` is the
    ///   `default_in_place` row and it IS the row that sentence is about — *"§3.3's row … silently
    ///   substituted the drop-free set"*. What the interval rule actually caught was a SECOND,
    ///   uncited referent in the same bullet: the quotation *"exactly 1 alloc + 1 free"* belongs to
    ///   gate 2, whose row is `:1221`. Repointing `:1220` at `:1221` would have swapped a correct
    ///   citation for a plausible sibling row — G2's shape exactly. The repair was to give the
    ///   second referent its own anchor and leave `:1220` alone. **A measured `actual` column
    ///   answers "where are these words", never "what should this anchor say".**
    const KNOWN_STALE: &[(&str, usize, &str, usize, usize)] = &[];

    let scans = scan_all();
    let mut report = String::new();
    let mut failed = false;
    let mut matched = vec![false; KNOWN_STALE.len()];

    for (doc, scan) in &scans {
        // The DENOMINATOR is printed first, and that ordering is the point: `UNQUOTED_MAX`
        // publishes only the un-checkable count, which reads as a residue until it is set beside
        // the total it is a share of.
        println!(
            "docs/{doc}: {} doc-to-doc anchor(s), {} content-checked, {} bounds-only, {} failed \
             the quote check",
            scan.doc_to_doc,
            scan.doc_to_doc - scan.doc_unquoted.len(),
            scan.doc_unquoted.len(),
            scan.doc_quote_violations.len()
        );
        for v in &scan.doc_quote_violations {
            match KNOWN_STALE.iter().position(|&(d, cl, t, tl, _)| {
                d == *doc && cl == v.citing_line && t == v.target && tl == v.cited_line
            }) {
                Some(i) => {
                    matched[i] = true;
                    println!("  KNOWN (held for the documentation act):\n{}", v.message);
                }
                None => {
                    failed = true;
                    report.push_str(&format!("docs/{doc}:\n{}\n", v.message));
                }
            }
        }
    }

    for (i, &(d, cl, t, tl, actual)) in KNOWN_STALE.iter().enumerate() {
        if !matched[i] {
            failed = true;
            report.push_str(&format!(
                "docs/{d}:{cl}  `{t}:{tl}` is listed in KNOWN_STALE but no longer reports.\n\
                 \x20     If it was repaired (the words are at `:{actual}`), DELETE its row — a \
                 stale exemption is the same defect as a stale anchor.\n"
            ));
        }
    }

    assert!(
        !failed,
        "a doc-to-doc citation quotes text its cited lines do not contain.\n\
         The anchor is inside the file — that is all the old bounds check ever asserted — but it \
         is on the WRONG LINE, and a wrong line that reads plausibly is worse than one out of \
         range: the reader who follows it arrives somewhere that looks like an answer.\n\
         Re-measure the anchor (`grep -n` the quoted phrase in the target) and repair the NUMBER. \
         Do not delete the quotation to silence this — that removes the only evidence tying the \
         citation to its target and drops the anchor into the un-checkable population, which is \
         pinned by `doc_to_doc_anchors_without_a_quotation_are_pinned`.\n\
         {report}"
    );
}

/// Doc-to-doc anchors the content check cannot reach — reported and pinned.
///
/// A citation that quotes nothing, or quotes something its target does not contain, is
/// BOUNDS-CHECKED ONLY: exactly the reach the whole census had before
/// [`doc_to_doc_anchors_carry_the_text_they_quote`] existed. Pinning the size of that population
/// is what stops it from growing — the alternative is a gate whose coverage shrinks every time
/// someone drops a quotation, with no signal that it did.
#[test]
fn doc_to_doc_anchors_without_a_quotation_are_pinned() {
    /// Per-document ceiling on doc-to-doc anchors that fall back to bounds.
    const UNQUOTED_MAX: &[(&str, usize)] = &[
        ("ARCHITECTURE.md", 0),
        ("FEATURE_MAP.md", 0),
        ("MESHLET-VIRTUAL-GEOMETRY-PLAN.md", 2),
        ("REFLECTION-ANALYSIS.md", 1),
        ("REFLECTION-PLAN-BOUNDARY.md", 1),
        // The largest population by far, and the reason is structural rather than sloppy: CORE is
        // the document that cites the other four most, and a great many of those citations point
        // at a rung or a table row without reproducing its words.
        ("REFLECTION-PLAN-CORE.md", 30),
        ("REFLECTION-PLAN-ECS.md", 9),
        ("REFLECTION-PLAN-GATES.md", 0),
        ("SYSTEMS.md", 0),
        // The register corpus, which `GATED_DOCS` admitted on 2026-09-10 but which this census
        // never saw: this map is the reflection lane's, authored against the nine documents that
        // existed on its own branch. The six below are therefore a FIRST measurement, taken on the
        // merged tree - they are what the run reports, not a target. See `OVER_WAIVED_MAX`, the one
        // per-document map both sides had, for the same note in the line's own words.
        //
        // ⚠️ **55 -> 71 in the A7 merge; the line's 55 are unchanged and all 16 are A7's.** Nine
        // are the UI campaign's own bounds-only citations. Seven were REFUSED continuations - a
        // bare `:N` inheriting a document named lines above, capped at zero - that this merge
        // repaired by naming the document beside the anchor, which is what moves them from
        // uncounted into this ledger: four into `APP-HOST-PLAN.md`, and three re-derived from the
        // pre-split sprite plan into its S4 and S5 parts by exact text.
        ("OPEN-QUESTIONS.md", 71),
        ("AETHER-GAIA-REVISION-2026-08-29.md", 0),
        ("gaia/CAMPAIGN.md", 15),
        ("gaia/DECISIONS.md", 0),
        ("gaia/LANGUAGE.md", 0),
        ("gaia/PENDING-SYNTAX-PLAN.md", 0),
    ];

    let scans = scan_all();
    let mut report = String::new();
    let mut failed = false;

    for (doc, scan) in &scans {
        let n = scan.doc_unquoted.len();
        let cap = UNQUOTED_MAX
            .iter()
            .find(|(d, _)| d == doc)
            .map(|(_, c)| *c)
            .unwrap_or_else(|| panic!("docs/{doc} has no entry in UNQUOTED_MAX"));
        println!("docs/{doc}: {n} doc-to-doc anchor(s) bounds-checked only (cap {cap})");
        if n > cap {
            failed = true;
            report.push_str(&format!(
                "docs/{doc}: {n} unquoted, cap {cap}\n{}\n",
                scan.doc_unquoted.join("\n")
            ));
        }
    }

    assert!(
        !failed,
        "more doc-to-doc anchors are bounds-checked-only than the pinned ceiling.\n\
         Each one is an anchor whose only assertion is `N <= line_count` — which is what let \
         `REFLECTION-PLAN-ECS.md:1851` stand at three sites for a sentence that lives at `:1853`. \
         Quote the target beside the citation instead of raising this ceiling: raising it buys a \
         green by shrinking the gate.\n{report}"
    );
}

/// Anchors that name their target by an ALIAS — reported and pinned.
///
/// `` `GATES:1316-1321` `` names a document, but `GATES` is not a file name, so the binder cannot
/// turn it into a path and the anchor inherits whatever the section last bound. That is a silent
/// inherit of exactly the shape that let a citation be checked against the wrong document, and the
/// reason this is a REPORT rather than a refusal is measurement, not leniency:
///
/// * The one live instance is `REFLECTION-PLAN-CORE.md:850`, and its inherited target is
///   `REFLECTION-PLAN-GATES.md` — bound thirteen lines above by §D31's own markdown link. The
///   alias resolves to the file the section already named, `:1316-1321` really does carry the
///   `` `let _ = <MyComp as boyko_reflect::Reflect>::TYPE_INFO;` `` body the sentence is about, and
///   **refusing the inherit would delete a correct check rather than add one.**
/// * Skipping it would also move it into no population at all: an unbound anchor is not counted by
///   [`unbindable_fragments_are_reported_and_pinned`], which counts only fragments the binder
///   RECOGNISED and could not resolve.
///
/// So the contract is "counted, never silent". If this number grows, the growth is a document
/// inventing more aliases, and each new one must be read against its inherited target before the
/// ceiling moves — the check the count exists to force.
#[test]
fn aliases_that_inherit_a_binding_are_reported_and_pinned() {
    /// Per-document ceiling on anchors bound through an alias.
    const ALIAS_BOUND_MAX: &[(&str, usize)] = &[
        ("ARCHITECTURE.md", 0),
        ("FEATURE_MAP.md", 0),
        ("MESHLET-VIRTUAL-GEOMETRY-PLAN.md", 0),
        ("REFLECTION-ANALYSIS.md", 0),
        ("REFLECTION-PLAN-BOUNDARY.md", 0),
        // `GATES:1316-1321` at `:850`. Read against its inherited target and correct — see above.
        ("REFLECTION-PLAN-CORE.md", 1),
        ("REFLECTION-PLAN-ECS.md", 0),
        ("REFLECTION-PLAN-GATES.md", 0),
        ("SYSTEMS.md", 0),
        // The register corpus, which `GATED_DOCS` admitted on 2026-09-10 but which this census
        // never saw: this map is the reflection lane's, authored against the nine documents that
        // existed on its own branch. The six below are therefore a FIRST measurement, taken on the
        // merged tree - they are what the run reports, not a target. See `OVER_WAIVED_MAX`, the one
        // per-document map both sides had, for the same note in the line's own words.
        ("OPEN-QUESTIONS.md", 0),
        ("AETHER-GAIA-REVISION-2026-08-29.md", 0),
        ("gaia/CAMPAIGN.md", 0),
        ("gaia/DECISIONS.md", 0),
        ("gaia/LANGUAGE.md", 0),
        ("gaia/PENDING-SYNTAX-PLAN.md", 0),
    ];

    let scans = scan_all();
    let mut report = String::new();
    let mut failed = false;

    for (doc, scan) in &scans {
        let n = scan.alias_bound.len();
        let cap = ALIAS_BOUND_MAX
            .iter()
            .find(|(d, _)| d == doc)
            .map(|(_, c)| *c)
            .unwrap_or_else(|| panic!("docs/{doc} has no entry in ALIAS_BOUND_MAX"));
        println!("docs/{doc}: {n} anchor(s) bound through an alias (cap {cap})");
        for a in &scan.alias_bound {
            println!("{a}");
        }
        if n > cap {
            failed = true;
            report.push_str(&format!(
                "docs/{doc}: {n} alias-bound, cap {cap}\n{}\n",
                scan.alias_bound.join("\n")
            ));
        }
    }

    assert!(
        !failed,
        "more anchors are bound through an alias than the pinned ceiling.\n\
         An alias is the document naming a target the binder cannot resolve, so the anchor is \
         checked against whatever the section last bound. Write the file name — \
         `REFLECTION-PLAN-GATES.md:1316-1321`, not `GATES:1316-1321` — rather than raising this \
         ceiling. If you do raise it, READ the reported inherited target first: the check the \
         anchor got is a check against THAT file.\n{report}"
    );
}

/// `<!-- doc-anchor-ignore -->` is the strongest opt-out in this file. This is its ledger.
///
/// ⚠️ **It was the only escape hatch here with no count and no test, and it is strictly stronger
/// than the one this file deliberately refused to allow.** The `~` waiver is kept out of the
/// doc-to-doc content check on the stated grounds that letting it through *"would hand every
/// doc-to-doc citation a one-character opt-out of the only thing that is ever asserted about it"*.
/// The marker does more than that: [`scan_text`] drops the whole LINE before any anchor on it is
/// scanned, so a marked line is invisible to the content check, to the shape check, to the path
/// check, and — until this test — to every count in the file. MEASURED: reverting the anchor at
/// `REFLECTION-PLAN-CORE.md:1047` to the G2 defect's stale number reddens the census with exit 101;
/// appending the marker to that same line returns exit 0 and `ok. 10 passed`, and no printed number
/// moves.
///
/// Every other hatch is pinned — `~` by [`waivers_that_were_not_needed_are_reported_and_pinned`],
/// unresolvable fragments by [`unbindable_fragments_are_reported_and_pinned`], quotation-free
/// doc-to-doc anchors by [`doc_to_doc_anchors_without_a_quotation_are_pinned`], the planned-path
/// marker by [`planned_paths_are_reported_and_pinned`] on BOTH halves of its contract. This one now
/// is too, at the live count with zero headroom.
///
/// (The stale number used in that measurement is the one named throughout this file as the G2
/// defect — the `REFLECTION-PLAN-ECS.md` line that three CORE sites cited for a sentence one line
/// below it. It is written here without a citation of its own so that this paragraph's prose does
/// not itself become an anchor into CORE, which is where a bare `:N` after a CORE mention would
/// bind — see [`md_citations_in_rust_sources`].)
///
/// The ceiling counts LINES, not markers, because the skip is per line: two markers on one line
/// silence one line's worth of checks.
#[test]
fn anchor_ignore_markers_are_reported_and_pinned() {
    /// Per-document ceiling on lines carrying [`IGNORE_MARKER`].
    const IGNORE_MARKER_MAX: &[(&str, usize)] = &[
        ("ARCHITECTURE.md", 0),
        ("FEATURE_MAP.md", 0),
        // ⚠️ This read `2`, for *"two `Lands.` declarations quoting paths that were built and then
        // moved"*, and the second half was false: both paths are on disk under the names the
        // declarations write. Both markers therefore silenced NOTHING — found the moment
        // [`ignore_markers_are_measured_by_what_they_silence`] measured content instead of lines —
        // and both are gone. A ceiling on how many lines carry an opt-out cannot tell a live
        // waiver from a dead one.
        ("MESHLET-VIRTUAL-GEOMETRY-PLAN.md", 0),
        ("REFLECTION-ANALYSIS.md", 2),
        ("REFLECTION-PLAN-BOUNDARY.md", 0),
        // The largest population, and it is one block: §D-audit's re-measurement report quotes the
        // BEFORE numbers of anchors it repaired, so every quoted `:N` in it is deliberately stale.
        //
        // ⚠️ **11, not the 12 that `grep -c doc-anchor-ignore` reports, and the difference is a
        // free line of headroom this ceiling must not carry.** `REFLECTION-PLAN-CORE.md:3327`
        // discusses the marker in prose — backticked `doc-anchor-ignore`, without the comment
        // delimiters — so it matches the substring and is not the marker. MEASURED: seeded from
        // grep, this ceiling sat one above the live count, and an injected marker silencing a real
        // stale anchor was absorbed by that slack and reported nothing. A ceiling for an opt-out
        // has to come from the SCANNER, which is the only thing that agrees with what the opt-out
        // actually does.
        //
        // 10, not 11: one of the eleven carried no citation at all and silenced nothing, and it
        // came off with the two above.
        ("REFLECTION-PLAN-CORE.md", 10),
        ("REFLECTION-PLAN-ECS.md", 0),
        ("REFLECTION-PLAN-GATES.md", 0),
        ("SYSTEMS.md", 0),
        // The register corpus, which `GATED_DOCS` admitted on 2026-09-10 but which this census
        // never saw: this map is the reflection lane's, authored against the nine documents that
        // existed on its own branch. The six below are therefore a FIRST measurement, taken on the
        // merged tree - they are what the run reports, not a target. See `OVER_WAIVED_MAX`, the one
        // per-document map both sides had, for the same note in the line's own words.
        //
        // ⚠️ **522 -> 550 in the A7 merge: 21 are A7's own and 7 were added by the merge, each on
        // a line that quotes a stale coordinate ON PURPOSE** - a pre-split sprite-plan coordinate
        // (four lines), a quotation of what that plan said (one), a citation deliberately
        // repointed past the end of a file to prove a gate vacuous (one), and a coordinate its
        // own entry calls "dead, and dead pre-split" (one). The alternative for each was to re-aim
        // a number the sentence records as historical, which is the falsification this marker
        // exists to prevent. The line's 522 are unchanged.
        ("OPEN-QUESTIONS.md", 550),
        ("AETHER-GAIA-REVISION-2026-08-29.md", 0),
        ("gaia/CAMPAIGN.md", 0),
        ("gaia/DECISIONS.md", 0),
        ("gaia/LANGUAGE.md", 0),
        ("gaia/PENDING-SYNTAX-PLAN.md", 0),
    ];

    let scans = scan_all();
    let mut report = String::new();
    let mut failed = false;

    for (doc, scan) in &scans {
        let n = scan.ignored_lines.len();
        let cap = IGNORE_MARKER_MAX
            .iter()
            .find(|(d, _)| d == doc)
            .map(|(_, c)| *c)
            .unwrap_or_else(|| panic!("docs/{doc} has no entry in IGNORE_MARKER_MAX"));
        println!("docs/{doc}: {n} line(s) carry `{IGNORE_MARKER}` (cap {cap})");
        for l in &scan.ignored_lines {
            println!("{l}");
        }
        if n > cap {
            failed = true;
            report.push_str(&format!(
                "docs/{doc}: {n} ignored line(s), cap {cap}\n{}\n",
                scan.ignored_lines.join("\n")
            ));
        }
    }

    assert!(
        !failed,
        "more lines carry `{IGNORE_MARKER}` than the pinned ceiling.\n\
         The marker drops the WHOLE LINE from every check in this file — anchors, path mentions \
         and quotations alike — so each one is a line nothing in this census reads. It is for a \
         line that quotes a stale anchor ON PURPOSE, not for a line that is merely failing: a \
         failing anchor is repaired by re-measuring the number.\n{report}"
    );
}

/// The marker silences a REAL violation, and the ledger above sees it when it does.
///
/// Both halves are asserted against a document built here, because the property is about what the
/// marker does to a violation and the corpus is (correctly) free of one. Without the red half this
/// would be a test that the marker counts lines; with it, it is a test that the count tracks the
/// thing the count exists to bound.
#[test]
fn an_ignore_marker_that_silences_a_real_violation_is_visible_in_the_count() {
    // 90000 is past the end of a manifest that exists, so the violation is the anchor's alone —
    // no missing path, no ambiguous fragment.
    const CITE: &str = "The kernel manifest `crates/boyko_ecs/Cargo.toml:90000` is past its end.";

    let red = scan_text("SYNTHETIC.md", CITE);
    assert_eq!(
        red.ignored_lines.len(),
        0,
        "unmarked line must not enter the ignore ledger"
    );
    assert_eq!(
        red.anchor_violations.len(),
        1,
        "the synthetic citation must actually violate, or the green half below proves nothing; \
         got {:?}",
        red.anchor_violations
    );

    let silenced = scan_text("SYNTHETIC.md", &format!("{CITE} {IGNORE_MARKER}"));
    assert!(
        silenced.anchor_violations.is_empty(),
        "the marker is supposed to silence the whole line; it did not: {:?}",
        silenced.anchor_violations
    );
    assert_eq!(
        silenced.anchors, 0,
        "a marked line's anchors are not merely excused, they are never counted"
    );
    assert_eq!(
        silenced.ignored_lines.len(),
        1,
        "a marker that silences a real violation MUST be visible in the ledger, or the ceiling in \
         `anchor_ignore_markers_are_reported_and_pinned` bounds nothing"
    );
    // ⚠️ The half above is the LINE count, and it does not move when the marked line gains
    // another violation — MEASURED, see `DocScan::silenced`. This is the half that does.
    assert_eq!(
        silenced.silenced.len(),
        1,
        "the marker's ledger must measure WHAT it silences, not only that a line carries it; got \
         {:?}",
        silenced.silenced
    );
    assert!(
        silenced.stale_ignored.is_empty(),
        "a marker over a real violation is not stale: {:?}",
        silenced.stale_ignored
    );

    // And the second half of the contract, the one `PLANNED_MARKER` has had all along: a marker
    // that waives nothing is itself a violation, because it is a slot left open for whatever the
    // next edit writes on that line.
    let empty = scan_text(
        "SYNTHETIC.md",
        &format!("Nothing here cites anything. {IGNORE_MARKER}"),
    );
    assert_eq!(
        empty.silenced.len(),
        0,
        "this line has nothing to silence; got {:?}",
        empty.silenced
    );
    assert_eq!(
        empty.stale_ignored.len(),
        1,
        "a marker that silences nothing must be reported, exactly as a `{PLANNED_MARKER}` over a \
         file that exists is"
    );

    // The measurement is line-local and must stay so: a marked line is re-scanned with its OWN
    // sticky binding restored beside it, and two violations on one line are two, not one.
    let two = scan_text(
        "SYNTHETIC.md",
        &format!(
            "{CITE} And `crates/boyko_ecs/no_such_file.rs` is not on disk. {IGNORE_MARKER}"
        ),
    );
    assert_eq!(
        two.silenced.len(),
        2,
        "appending a fresh violation to an already-marked line must MOVE the number; got {:?}",
        two.silenced
    );
}

/// A `Continuation` inherits its target, and this pins WHERE that target may be named.
///
/// # The measurement that chose refusal over counting
///
/// The bare `(:N)` form is legitimate and dense: it is how every member table in these documents
/// is written, and refusing the form outright would delete the densest citations in the corpus.
/// So the question is narrower — how many continuations inherit a target named on an EARLIER line,
/// and what do they inherit?
///
/// MEASURED 2026-08-28 over the nine gated documents: **373 continuations inherit across a line
/// boundary. 370 of them inherit a SOURCE file — the `**File:** foo.rs` header with a member list
/// under it, which is the binding this gate was built for and which cannot be refused without
/// losing hundreds of live checks. Exactly 3 inherit a DOCUMENT, and all three are in
/// `REFLECTION-PLAN-CORE.md`.** Three is a handful, so this population is refused rather than
/// counted, and the anchors are rewritten to name their document beside the number.
///
/// # Why counting them would have bought nothing
///
/// A ledger that counts anchors does not move when a counted anchor's NUMBER changes, which is the
/// same defect [`DocScan::silenced`] exists to close for the ignore marker. All three sites were
/// wrong in a way a count could not have shown:
///
/// * one cited a fenced sketch that is in CORE and inherited `REFLECTION-ANALYSIS.md` from the
///   previous line, so the gate's verdict was *"quotes nothing this document contains; bounds
///   only"* — a FALSE NEGATIVE filed inside the population the census publishes as un-checkable;
/// * one inherited the right document and named lines that hold an unrelated subject;
/// * one inherited CORE and named lines that hold a run ledger, while the sentence's own claim is
///   recorded at a different range and is cited correctly from a second site in the same document.
///
/// Refusing binds each of them to a name the reader can check, and the first became
/// content-checked rather than merely in-bounds.
#[test]
fn continuations_do_not_inherit_a_document_across_a_line_boundary() {
    /// Per-document ceiling on continuations that inherit a DOCUMENT from an earlier line.
    ///
    /// Zero everywhere, and it is a ceiling on a refusal rather than on a check: such an anchor is
    /// not examined at all, so a non-zero row would be coverage lost, not a smell tolerated.
    const CROSS_LINE_DOC_MAX: usize = 0;

    let scans = scan_all();
    let mut report = String::new();
    let mut total_src = 0usize;

    for (doc, scan) in &scans {
        total_src += scan.cross_line_src;
        println!(
            "docs/{doc}: {} continuation(s) inherit a document across a line (cap \
             {CROSS_LINE_DOC_MAX}), {} inherit a source file",
            scan.cross_line_doc.len(),
            scan.cross_line_src
        );
        for c in &scan.cross_line_doc {
            println!("{c}");
        }
        if scan.cross_line_doc.len() > CROSS_LINE_DOC_MAX {
            report.push_str(&format!("docs/{doc}:\n{}\n", scan.cross_line_doc.join("\n")));
        }
    }
    println!("{total_src} source-file continuation(s) inherit across a line boundary in total");

    // The corpus is (now) free of the shape, so the discriminator is asserted directly. Without
    // this, a cap of zero over an empty population is a gate that cannot fail — the defect this
    // campaign keeps finding, and one this file has now committed twice.
    let across = scan_text(
        "SYNTHETIC.md",
        "The rule is in [g](../docs/REFLECTION-PLAN-GATES.md).\nIt is stated at `:1321-1323`.",
    );
    assert_eq!(
        across.cross_line_doc.len(),
        1,
        "a `:N` inheriting a document from the previous line must be refused; got {:?}",
        across.cross_line_doc
    );
    let beside = scan_text(
        "SYNTHETIC.md",
        "The rule is in [g](../docs/REFLECTION-PLAN-GATES.md), stated at `:1321-1323`.",
    );
    assert!(
        beside.cross_line_doc.is_empty(),
        "a document named on the anchor's OWN line is the legitimate case and must still bind: \
         {:?}",
        beside.cross_line_doc
    );
    let source = scan_text(
        "SYNTHETIC.md",
        "**File:** [s](../crates/boyko_ecs/src/lib.rs)\nIts first line is `:1`.",
    );
    assert_eq!(
        source.cross_line_src, 1,
        "the member-table shape — a source file bound by a header, members listed under it — is \
         the 377 this refusal deliberately does NOT touch"
    );
    assert!(source.cross_line_doc.is_empty());

    // ⚠️ **The same two shapes INSIDE A FENCE, and they are asserted here because the live
    // population of the first is ZERO.** The fenced arm's `Continuation` branch was an empty
    // block: neither refusal nor count, so a bare `:N` under a fence opened on a document was
    // checked against that document with nothing said, and a bare `:N` under a fence opened on a
    // source file was checked and left out of the total. A cap of zero over an empty population is
    // a gate that cannot fail — the defect this file has now committed three times — so the fence
    // discriminator is built here rather than waited for.
    let fenced_doc = scan_text(
        "SYNTHETIC.md",
        "The rule is in [g](../docs/REFLECTION-PLAN-GATES.md).\n```\nIt is stated at `:1321-1323`.\n```",
    );
    assert_eq!(
        fenced_doc.cross_line_doc.len(),
        1,
        "a `:N` inside a fence inheriting a DOCUMENT bound at the fence's opening line must be \
         refused exactly as it is outside one; got {:?}",
        fenced_doc.cross_line_doc
    );
    let fenced_src = scan_text(
        "SYNTHETIC.md",
        "**File:** [s](../crates/boyko_ecs/src/lib.rs)\n```\nIts first line is `:1`.\n```",
    );
    assert_eq!(
        fenced_src.cross_line_src, 1,
        "a `:N` inside a fence inheriting a SOURCE file is the counted case, and it was missing \
         from the total entirely — the printed figure was 370 while the tree held 377"
    );
    assert!(fenced_src.cross_line_doc.is_empty());
    // And the legitimate case still binds: a margin note that names its own file on its own line
    // is the shape fences exist for, and it must not be swept into the refusal.
    // Assembled rather than written out: a literal citation here would enter the `.rs` → `.rs`
    // census's own population, which reads this file. MEASURED — the first draft moved its
    // string-literal count by one.
    let named_fixture = format!(
        "```\n`crates/boyko_ecs/src/lib{}rs:1` is the first line, and `:1` again.\n```",
        '.'
    );
    let fenced_named = scan_text("SYNTHETIC.md", &named_fixture);
    assert!(
        fenced_named.cross_line_doc.is_empty() && fenced_named.cross_line_src == 0,
        "a fence whose margin note names its target on the anchor's OWN line binds same-line and \
         is neither refused nor counted: {:?} / {}",
        fenced_named.cross_line_doc,
        fenced_named.cross_line_src
    );

    assert!(
        report.is_empty(),
        "a `:N` continuation inherited a DOCUMENT named on an earlier line.\n\
         The sticky binding models a member table: one `**File:**` header, a source file, and a \
         list of members under it. A doc-to-doc citation is not written that way, and every \
         misbinding this census has found came from one inheriting a document it never named — \
         including one that was then filed as `bounds only` while the citing document held the \
         quoted words at the very numbers it wrote. Name the document beside the anchor \
         (`REFLECTION-PLAN-GATES.md:1321-1323`, not a bare `:1321-1323` under a mention two lines \
         up) rather than raising this ceiling.\n{report}"
    );
}

/// The false-negative class the misbinding above produced, made visible.
///
/// An anchor reported as *"quotes nothing this document contains; bounds only"* is the census
/// saying it could not reach that anchor. When the CITING document carries the quoted words at the
/// very coordinates the anchor names, that verdict is wrong: the instrument reached the anchor and
/// answered against the wrong file. Left in the bounds-only population it reads as discipline —
/// `UNQUOTED_MAX` sits at the live count with zero headroom for every document — and it is not
/// discipline, it is a check that silently did not happen.
///
/// Pinned at zero. A hit is not proof on its own (two documents may share a sentence), so the
/// report names the coordinates and leaves the reading to a person; but it is the one piece of
/// evidence that separates *un-checkable* from *misbound*, and the population it bounds is the one
/// the previous audit round named as where the next hole would be.
#[test]
fn doc_to_doc_anchors_whose_words_are_at_their_own_numbers_are_reported() {
    let scans = scan_all();
    let mut report = String::new();

    for (doc, scan) in &scans {
        println!(
            "docs/{doc}: {} doc-to-doc anchor(s) whose words sit at their own numbers in THIS \
             document (cap 0)",
            scan.doc_misbound.len()
        );
        for m in &scan.doc_misbound {
            println!("{m}");
        }
        if !scan.doc_misbound.is_empty() {
            report.push_str(&format!("docs/{doc}:\n{}\n", scan.doc_misbound.join("\n")));
        }
    }

    // A cap of zero over an empty population proves nothing, so the discriminator is asserted on a
    // document built here. The citing name is a REAL one — `scan_text` reads the citing document
    // off disk for this check — and the numbers are that document's own, pointed at a sibling that
    // does not contain the words. That is the misbinding shape, minus the misbinding.
    let mis = scan_text(
        "REFLECTION-PLAN-CORE.md",
        "The sketch is at `REFLECTION-PLAN-GATES.md:341-349`, introduced by \
         *\"where `boyko_reflect` declares\"*.",
    );
    assert_eq!(
        mis.doc_misbound.len(),
        1,
        "an anchor whose words sit at its own numbers in the CITING document must be reported as \
         misbound rather than filed as un-checkable; got {:?} / unquoted {:?}",
        mis.doc_misbound,
        mis.doc_unquoted
    );
    // And the control: the same words, cited from a document that does NOT carry them at those
    // numbers, stay in the bounds-only population where they belong.
    let ok = scan_text(
        "REFLECTION-PLAN-ECS.md",
        "The sketch is at `REFLECTION-PLAN-GATES.md:341-349`, introduced by \
         *\"where `boyko_reflect` declares\"*.",
    );
    assert!(
        ok.doc_misbound.is_empty(),
        "the citing document does not carry these words at these numbers, so there is no evidence \
         of a misbinding and none must be claimed: {:?}",
        ok.doc_misbound
    );

    assert!(
        report.is_empty(),
        "a doc-to-doc anchor was checked against a document that does not carry its quotation, \
         while the citing document carries it at exactly the line the anchor names.\n\
         That is a misbinding wearing the `bounds only` verdict. Either the citation means its own \
         document and should say so, or the numbers belong to the named target and are wrong.\n\
         {report}"
    );
}

/// The `<!-- doc-anchor-ignore -->` ledger, measured by WHAT it silences.
///
/// ⚠️ **[`anchor_ignore_markers_are_reported_and_pinned`] counts LINES, and a marked line's content
/// is unbounded.** MEASURED 2026-08-28: appending a real anchor violation and a real path
/// violation to `REFLECTION-PLAN-CORE.md:2128`, a line that already carries the marker, left the
/// suite at `exit 0`, `ok. 14 passed`, and every printed number unmoved — including that line
/// ceiling, which still read `11 line(s) carry the marker (cap 11)`. The same citation on an
/// unmarked line reds at exit 101. A ceiling on how many lines carry an opt-out bounds nothing
/// about how much each one takes out of the census, so a marked line is a permanently open slot.
///
/// [`PLANNED_MARKER`] has had both halves of its contract gated since [`DocScan::stale_planned`]
/// landed: one test counts what it waives, and the same test fails a marker that waives nothing.
/// This is those two halves for the strictly stronger opt-out.
///
/// The count is EXACT, not a ceiling, for the reason `PLANNED_EXACT` is: a marker's whole purpose
/// is to be removed when what it silences is repaired, and a ceiling cannot tell a repair from a
/// silence that merely stopped being reported.
#[test]
fn ignore_markers_are_measured_by_what_they_silence() {
    /// Exact count of violations each document's [`IGNORE_MARKER`] lines take out of the census.
    const SILENCED_EXACT: &[(&str, usize)] = &[
        ("ARCHITECTURE.md", 0),
        ("FEATURE_MAP.md", 0),
        // Both markers here were placed over `Lands.` declarations whose files did not exist yet.
        // Both files exist now, so both markers waived nothing and are gone; see the run that
        // found them in this test's own history.
        ("MESHLET-VIRTUAL-GEOMETRY-PLAN.md", 0),
        // The two `(3)` spans of the refusal-matrix bullet, which the bare-parenthesised form
        // reads as anchors against the section's binding.
        ("REFLECTION-ANALYSIS.md", 2),
        ("REFLECTION-PLAN-BOUNDARY.md", 0),
        // §D-audit's re-measurement report, which quotes the BEFORE numbers of anchors it
        // repaired, plus two older quotations. Every one is a `reflect_on.rs` anchor from before
        // that file was cut down to 88 lines, which is what makes them deliberately stale.
        ("REFLECTION-PLAN-CORE.md", 26),
        ("REFLECTION-PLAN-ECS.md", 0),
        ("REFLECTION-PLAN-GATES.md", 0),
        ("SYSTEMS.md", 0),
        // The register corpus, which `GATED_DOCS` admitted on 2026-09-10 but which this census
        // never saw: this map is the reflection lane's, authored against the nine documents that
        // existed on its own branch. The six below are therefore a FIRST measurement, taken on the
        // merged tree - they are what the run reports, not a target. See `OVER_WAIVED_MAX`, the one
        // per-document map both sides had, for the same note in the line's own words.
        //
        // 743 -> 798 in the A7 merge, every one of the 55 on a line the merge inserted: what A7's
        // 21 markers and the 7 the merge added silence, measured by the scan rather than
        // estimated. The line's 743 are unchanged.
        //
        // 798 -> 797 in the A8 merge (the `integ/unified` cut): the marker over the pass-8
        // checkpoint line silenced one finding, "path does not exist" for
        // `docs/unification/checkpoint-2026-09-11/msvc-citations-pass8-work-order.md`, a file
        // that lived only on `feat/multi-paradigm-render`. The merge brought the file in, so the
        // marker silenced nothing and came off in the same commit.
        ("OPEN-QUESTIONS.md", 797),
        ("AETHER-GAIA-REVISION-2026-08-29.md", 0),
        ("gaia/CAMPAIGN.md", 0),
        ("gaia/DECISIONS.md", 0),
        ("gaia/LANGUAGE.md", 0),
        ("gaia/PENDING-SYNTAX-PLAN.md", 0),
    ];

    /// Exact count, per document, of `<!-- doc-anchor-ignore -->` markers that silence NOTHING.
    ///
    /// Zero is the right pin wherever it holds, and it holds for every document this census was
    /// written against: a marker over a line with no violation is a slot left open for the next
    /// citation written beside it, which is the hazard the panic message describes.
    ///
    /// WARNING: `OPEN-QUESTIONS.md` enters at 140, and that is a FIRST MEASUREMENT rather than a
    /// concession. This census is the reflection lane's; `OPEN-QUESTIONS.md` is in `GATED_DOCS` by
    /// the line's rung, so the two met for the first time in the A6 merge and nothing had ever
    /// counted this population. The 140 sit in the KE16/VG audit narrative, where the prose is
    /// rewritten constantly and the markers were applied defensively. They are PINNED rather than
    /// deleted because the deletion is not obviously safe: a marked line is skipped ENTIRELY, so
    /// removing its marker also restores its path mentions to the sticky-target chain and can move
    /// findings on LATER lines. Lower it with a measurement, one section at a time - the pin is
    /// EXACT, so the population cannot grow unnoticed either.
    const STALE_MARKER_EXACT: &[(&str, usize)] = &[
        ("ARCHITECTURE.md", 0),
        ("FEATURE_MAP.md", 0),
        ("MESHLET-VIRTUAL-GEOMETRY-PLAN.md", 0),
        ("REFLECTION-ANALYSIS.md", 0),
        ("REFLECTION-PLAN-BOUNDARY.md", 0),
        ("REFLECTION-PLAN-CORE.md", 0),
        ("REFLECTION-PLAN-ECS.md", 0),
        ("REFLECTION-PLAN-GATES.md", 0),
        ("SYSTEMS.md", 0),
        // 140 -> 141 in the A7 merge. The one is A7's "The options" bullet, which names the
        // marker in prose and is therefore read as a marker silencing nothing - the class most of
        // the 140 belong to, pinned for the reason the doc comment above gives.
        ("OPEN-QUESTIONS.md", 141),
        ("AETHER-GAIA-REVISION-2026-08-29.md", 0),
        ("gaia/CAMPAIGN.md", 0),
        ("gaia/DECISIONS.md", 0),
        ("gaia/LANGUAGE.md", 0),
        ("gaia/PENDING-SYNTAX-PLAN.md", 0),
    ];

    let scans = scan_all();
    let mut report = String::new();

    for (doc, scan) in &scans {
        let n = scan.silenced.len();
        let want = SILENCED_EXACT
            .iter()
            .find(|(d, _)| d == doc)
            .map(|(_, c)| *c)
            .unwrap_or_else(|| panic!("docs/{doc} has no entry in SILENCED_EXACT"));
        println!("docs/{doc}: {n} violation(s) silenced by `{IGNORE_MARKER}` (pinned {want})");
        for s in &scan.silenced {
            println!("{s}");
        }
        for s in &scan.stale_ignored {
            println!("{s}");
        }
        if n != want {
            report.push_str(&format!(
                "docs/{doc}: {n} silenced, pinned {want}\n{}\n",
                scan.silenced.join("\n")
            ));
        }
        let stale_want = STALE_MARKER_EXACT
            .iter()
            .find(|(d, _)| d == doc)
            .map(|(_, c)| *c)
            .unwrap_or_else(|| panic!("docs/{doc} has no entry in STALE_MARKER_EXACT"));
        if scan.stale_ignored.len() != stale_want {
            report.push_str(&format!(
                "docs/{doc}: {} marker(s) that silence nothing, pinned {stale_want}\n{}\n",
                scan.stale_ignored.len(),
                scan.stale_ignored.join("\n")
            ));
        }
    }

    // ⚠️ **The eleven classes are asserted here, because the corpus exercises three of them.**
    // `silenced_by` extended from `path_violations`, `anchor_violations` and
    // `doc_quote_violations` only, and the eight it dropped included every class a marked line is
    // most likely to hide. A pin over a corpus that happens not to contain the other eight is a
    // gate that cannot fail, so the discriminators are built rather than waited for.
    let src_sticky = repo_root().join("crates/boyko_macros/src/lib.rs");
    // Assembled, never written out: this file is inside the `.rs` → `.rs` census's own population,
    // and a literal probe would move that census's published figures. MEASURED — the first draft
    // took its string-literal count from 5 to 6.
    let unbindable = silenced_by(
        "SYNTHETIC.md",
        &format!("A citation of `no_such_file_probe{}rs:5` here. {IGNORE_MARKER}", '.'),
        Some(&src_sticky),
    );
    assert!(
        unbindable.iter().any(|w| w.contains("names no single file")),
        "a marked line hiding an UNBINDABLE fragment must be reported: {unbindable:?}"
    );

    // The cross-line refusal is the one class the same-line arrangement cannot produce —
    // `set_line == lineno` by construction — and it is why `silenced_by` runs two scans rather
    // than one. Asserted directly because NO marked line in the corpus currently inherits a
    // document, so the live count is zero and would prove nothing.
    let doc_sticky = repo_root().join("docs/REFLECTION-PLAN-GATES.md");
    let crossed = silenced_by(
        "SYNTHETIC.md",
        &format!("The rule is stated at `:1`. {IGNORE_MARKER}"),
        Some(&doc_sticky),
    );
    assert!(
        crossed.iter().any(|w| w.contains("inherits")),
        "a marked line hiding a continuation that inherits a DOCUMENT must be reported; this is \
         the class the single same-line re-scan could never see: {crossed:?}"
    );
    // And the control, so the second scan does not invent a refusal where there is none: the same
    // shape over a SOURCE sticky is the legitimate member-table case and stays silent.
    let not_crossed = silenced_by(
        "SYNTHETIC.md",
        &format!("The header is at `:1`. {IGNORE_MARKER}"),
        Some(&src_sticky),
    );
    assert!(
        !not_crossed.iter().any(|w| w.contains("inherits")),
        "a continuation inheriting a SOURCE file is counted, never refused, and must not appear \
         in the silenced ledger as a refusal: {not_crossed:?}"
    );

    assert!(
        report.is_empty(),
        "the `{IGNORE_MARKER}` ledger moved.\n\
         A marker is for a line that quotes a stale anchor ON PURPOSE. If this count went UP, a \
         marked line gained a citation nothing now reads — write it on its own line instead. If it \
         went DOWN, what the marker silenced has been repaired and the marker comes off in the \
         same commit. And a marker that silences NOTHING is a slot left open for the next citation \
         written beside it, exactly as a `{PLANNED_MARKER}` over a file that exists is.\n{report}"
    );
}

/// One `X.md:N` citation written inside a Rust source — the direction this census used to be blind
/// to in both halves.
struct MdCitation {
    /// Repo-relative path of the citing source, `/`-separated.
    file: String,
    /// Line of the citing source.
    line: usize,
    /// The target as the source names it.
    raw: String,
    /// The resolved target.
    target: PathBuf,
    start: usize,
    end: Option<usize>,
}

/// Every `X.md:N` citation written inside a `.rs` source, and the anchors that named no document.
///
/// # Binding is LINE-LOCAL, deliberately
///
/// [`scan_text`] carries a sticky target across lines because a document section has a subject: a
/// `**File:**` header, a heading that resets it. A run of `///` comment lines has no such
/// structure, and inheriting a target down a doc comment is how the forward direction produced its
/// misbindings. So a citation binds to a name written on its OWN line, or it is not checked and is
/// counted instead. MEASURED, and it is not hypothetical: `tests/internal_docs_anchors.rs:2783`
/// writes a bare `` `:1853` `` on a line whose only named document is `REFLECTION-PLAN-CORE.md`
/// while the sentence means `REFLECTION-PLAN-ECS.md`. CORE has 4026 lines, so a cross-line or
/// last-mention-wins binding would have checked it against the wrong document and PASSED.
fn md_citations_in_rust_sources() -> (Vec<MdCitation>, Vec<String>) {
    let root = repo_root();
    let mut found = Vec::new();
    let mut skipped = Vec::new();

    for path in repo_files() {
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        if !text.contains(".md") {
            continue;
        }
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");

        for (idx, line) in text.lines().enumerate() {
            if !line.contains(".md") {
                continue;
            }
            let (line_mentions, line_anchors) = scan_line(line, &docs_dir());
            if line_anchors.is_empty() {
                continue;
            }
            let mut current: Option<(String, PathBuf)> = None;
            let mut mi = 0usize;
            for anchor in &line_anchors {
                while mi < line_mentions.len() && line_mentions[mi].col < anchor.col {
                    let m = &line_mentions[mi];
                    if is_file_shaped(&m.raw) && m.resolved.is_file() {
                        current = Some((m.raw.clone(), m.resolved.clone()));
                    }
                    mi += 1;
                }
                // A name written at the anchor overrides, and an unresolvable one CLEARS rather
                // than inherits — the same rule the forward direction learned.
                let named = match fragment_before(line, anchor.col) {
                    LeftOfAnchor::Named(frag) => {
                        current = resolve_unique_fragment(frag).map(|p| (frag.to_string(), p));
                        true
                    }
                    LeftOfAnchor::Alias(_) => {
                        current = None;
                        false
                    }
                    LeftOfAnchor::Continuation => false,
                };
                match current
                    .as_ref()
                    .filter(|(_, p)| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("md")))
                {
                    Some((raw, target)) => found.push(MdCitation {
                        file: rel.clone(),
                        line: idx + 1,
                        raw: raw.clone(),
                        target: target.clone(),
                        start: anchor.line_no,
                        end: anchor.range_end,
                    }),
                    // An anchor that NAMED a non-`.md` file is a source citation, which the forward
                    // direction owns; it is not a miss. Only an UNNAMED anchor on a line that
                    // mentions a document is a citation this scan could not reach.
                    None if !named => skipped.push(format!(
                        "  {rel}:{}  `:{}` names no document on its own line",
                        idx + 1,
                        anchor.line_no
                    )),
                    None => {}
                }
            }
        }
    }
    (found, skipped)
}

/// `.md:N` citations written inside `.rs` sources are bounds-checked. They used to be read by
/// nothing at all.
///
/// ⚠️ **The gap was self-documenting and it was self-inflicting.** The comment above
/// [`ROOT_PREFIXES`] recorded that *"this census scans `.md` files for citations into `.rs`, never
/// the reverse"*, called the consequence *"a known gap, not scheduled"*, and gave as its example a
/// citation of its own that had *"already been off by two when written"*. The file then accumulated
/// more of them. Two were live when this test was added, both found by READING and neither by any
/// gate. This file's `:196` cited line 756 of `FEATURE_MAP.md:758` for that document's sole
/// parenthesised waiver `` `(65~` ``; its `:1003` cited line 2196 of
/// `REFLECTION-PLAN-ECS.md:2330` for *"three sibling documents [that] say …"*. Both are repaired,
/// and each is written here as a citation of the REPAIRED line with the stale number as a bare
/// quantity, so that recording the defect does not re-commit it.
///
/// # This is BOUNDS ONLY, and the measurement says why
///
/// The forward direction's content check would not have caught either defect, and that is measured
/// rather than assumed:
///
/// * `:2196`'s quotation contains `[` **and** `…`, so [`quotations`] rejects it twice over — by the
///   Interpolation rule and by the Elision rule. It would land in the un-checkable population.
/// * `:196` does not quote its target at all. It names a backticked literal, `` `(65~` ``, which is
///   not a `"`-delimited quotation and is below [`MIN_QUOTE_LEN`] besides.
///
/// So a content check bolted onto this direction would have been a gate that could not fail on the
/// only two defects present — the exact shape this campaign keeps finding. What bounds buys is the
/// class that actually rots in a corpus being rewritten by the hundred lines: a cited document that
/// shrinks below `N`, a range that ends past its end, and a name that resolves to no file at all.
/// Content in this direction stays UNCHECKED, and is stated here rather than implied.
#[test]
fn md_line_citations_written_inside_rust_sources_are_bounds_checked() {
    /// Per-source FLOOR on citations found. A floor, not a ceiling: adding a citation is normal,
    /// and losing one silently is how a scan goes vacuum-green. Lower it deliberately, with the
    /// reason, when a citation is genuinely deleted.
    const MD_CITATIONS_MIN: &[(&str, usize)] = &[
        ("crates/boyko_log/src/codes.rs", 1),
        ("crates/boyko_ui/src/layout.rs", 2),
        ("tests/internal_docs_anchors.rs", 25),
    ];
    /// Ceiling on anchors that named no document on their own line and were therefore not checked.
    ///
    /// All eight live entries were read, and none is a document citation the scan missed:
    ///
    /// * five are continuations of a SOURCE citation, or of nothing at all — a bare tail after a
    ///   `tags.rs` anchor, the prose about the parenthesised-enumerator false positive, this file's
    ///   own bare self-citation, and a bare tail naming a line of this file;
    /// * two are the CORE-mention-with-an-ECS-continuation shape this scan exists to REFUSE, and
    ///   refusing them is the point — see [`md_citations_in_rust_sources`];
    /// * one is in `crates/boyko_log/src/codes.rs`, where a zero-padding format specifier inside a
    ///   `format!` that builds an `.md` name presents its width to [`scan_line`] as an anchor. It
    ///   is unbound, so it is skipped and counted rather than checked against whatever a document
    ///   mention elsewhere on the line would have supplied — the same reason the forward direction
    ///   refuses rather than guesses.
    ///
    /// ⚠️ Written without reproducing any of their `:N` tokens **on purpose**: this scan reads this
    /// file, so an inventory that spelled its own entries out would enlarge the population it is
    /// bounding. MEASURED — the first draft of this comment took the count from 8 to 10.
    ///
    /// ⚠️ **8 -> 13 in the A6 merge, and the five are the CORPUS growing, not the check
    /// loosening.** This census reads every `.rs` in the tree; it was written on the reflection
    /// lane, and the merge brings the line's `tests/gaia_ruled_vs_open_census.rs` and its
    /// siblings under it for the first time. Re-derived from the merged tree's run.
    const UNBOUND_MAX: usize = 13;

    let (found, skipped) = md_citations_in_rust_sources();
    let mut report = String::new();

    let mut per_file: BTreeMap<&str, usize> = BTreeMap::new();
    for c in &found {
        *per_file.entry(c.file.as_str()).or_default() += 1;
    }
    println!("{} `.md:N` citation(s) inside .rs sources:", found.len());
    for c in &found {
        println!(
            "  {}:{}  ->  {}:{}{}",
            c.file,
            c.line,
            c.raw,
            c.start,
            c.end.map(|e| format!("-{e}")).unwrap_or_default()
        );
    }
    println!("{} anchor(s) named no document on their own line (cap {UNBOUND_MAX}):", skipped.len());
    for s in &skipped {
        println!("{s}");
    }

    for c in &found {
        let Ok(text) = std::fs::read_to_string(&c.target) else {
            report.push_str(&format!(
                "  {}:{}  `{}:{}` — target is unreadable\n",
                c.file, c.line, c.raw, c.start
            ));
            continue;
        };
        let total = text.lines().count();
        if c.start > total {
            report.push_str(&format!(
                "  {}:{}  `{}:{}` is past end of file ({total} lines)\n",
                c.file, c.line, c.raw, c.start
            ));
        }
        if let Some(end) = c.end {
            if end < c.start {
                report.push_str(&format!(
                    "  {}:{}  `{}:{}-{end}` ends before it starts\n",
                    c.file, c.line, c.raw, c.start
                ));
            } else if end > total {
                report.push_str(&format!(
                    "  {}:{}  `{}:{}-{end}` ends past end of file ({total} lines)\n",
                    c.file, c.line, c.raw, c.start
                ));
            }
        }
    }

    for (file, floor) in MD_CITATIONS_MIN {
        let n = per_file.get(file).copied().unwrap_or(0);
        if n < *floor {
            report.push_str(&format!(
                "  {file}: {n} citation(s) found, floor {floor} — the scan stopped seeing some\n"
            ));
        }
    }
    if skipped.len() > UNBOUND_MAX {
        report.push_str(&format!(
            "  {} anchor(s) bound to no document, cap {UNBOUND_MAX}:\n{}\n",
            skipped.len(),
            skipped.join("\n")
        ));
    }

    assert!(
        report.is_empty(),
        "a `.md:N` citation written inside a Rust source does not hold.\n\
         These are bounds-checked only (see this test's doc for the measurement behind that), so a \
         report here means the cited document no longer has that line at all — re-measure with \
         `grep -n` and repair the number.\n{report}"
    );
}

/// Is the anchor at `col` a claim this source makes about the tree, or data it builds?
///
/// ⚠️ **The distinction is not fussiness, and the measurement is this file's own.** Over the whole
/// tree there are 425 `.rs`-to-`.rs` line citations; 25 of them are not written in a comment, only
/// 5 of those resolve to a file, and **3 of those 5 are the fixture strings of
/// [`a_range_tail_is_captured_with_the_waiver_written_on_either_side`]** — a parser control whose
/// whole purpose is to hand `scan_line` a citation with numbers chosen for the parser rather than
/// for the file. Reading those as claims would put three permanent entries in a stale list that
/// could never go green, which is the opposite of what a pinned list is for.
///
/// So: inside a `//` comment a citation is always a claim — these sources quote prose freely, and a
/// quotation mark in a doc comment must not switch the check off. Outside one, a citation inside a
/// string literal is data. The literal test is the parity of `"` before the anchor, which is exact
/// for a literal that opens and closes on its own line and treats a continued one as a claim; both
/// of the two live continued-literal citations are claims, so that is the safe direction.
fn is_a_claim_about_the_tree(line: &str, col: usize) -> bool {
    // `get` rather than an index: `col` is a byte offset, and a slice that does not land on a
    // character boundary must cost coverage, never a panic. It falls to "claim", the safe side.
    let head = line.get(..col).unwrap_or("");
    head.contains("//") || head.bytes().filter(|b| *b == b'"').count() % 2 == 0
}

/// Every `foo.rs:N` citation one `.rs` source writes about another, bound **on its own line**.
///
/// # The direction nothing read
///
/// The forward census scans `.md` → `.rs`/`.md`. Its reverse
/// ([`md_citations_in_rust_sources`]) scans `.rs` → `.md`, and filters its targets to `.md` **by
/// construction**. `.rs` → `.rs` was therefore read by nothing at all, in either census, while the
/// text above [`ROOT_PREFIXES`] declared the reverse direction *"SCHEDULED AND LANDED"*.
///
/// # Binding is Named-only, which is stricter than the `.md` direction
///
/// A citation counts only when the file name is written at the anchor, never as a bare `:N`
/// continuation or a parenthesised `(N)`. Two reasons, and both are measurements rather than taste.
///
/// (Written without an example, on purpose. The first draft of this sentence spelled one out with a
/// real file name beside a real number, and **this scan read the sentence as a thirteenth broken
/// citation** — the doc comment describing the rule became a subject of it. MEASURED.)
/// Cross-line stickiness is what produced every misbinding this campaign has found, so it is not
/// reintroduced here. And a Rust source is dense with `(N)` spans that are argument lists and
/// tuple indices, not citations; the `.md` direction can afford to read them because a document is
/// prose throughout, and this one cannot.
///
/// The cost is stated rather than implied: a `.rs` file that names its subject once and then lists
/// `:N` members under it is not covered. No such file exists in this tree today.
fn rs_citations_in_rust_sources() -> (Vec<MdCitation>, usize, Vec<String>, Vec<String>) {
    let root = repo_root();
    let mut found = Vec::new();
    let mut data = 0usize;
    let mut unbindable: Vec<String> = Vec::new();
    let mut dead: Vec<String> = Vec::new();

    for path in repo_files() {
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");

        for (idx, line) in text.lines().enumerate() {
            // ⚠️ The prefilter asks for `.rs`, **not** for `.rs:`, and the difference is measured.
            // A raw `.rs:` substring test sits in front of the very binder that was repaired to
            // read a name through its delimiters, and it dropped every split form before that
            // binder ever ran: a backtick or a bold marker between the name and the colon puts no
            // `.rs:` on the line at all. MEASURED 2026-08-28 with a citation 99 437 lines past the
            // end of a 562-line file — backticked-name form and bold-name form both left the count
            // at 240 and exited 0, while the fully backticked form redded. The fragment must end
            // in `.rs` (checked below), so requiring `.rs` alone cannot drop a citation this scan
            // could otherwise bind. It is the same prefilter shape `md_citations_in_rust_sources`
            // uses, for the same reason.
            if !line.contains(".rs") {
                continue;
            }
            let (_, line_anchors) = scan_line(line, &docs_dir());
            for anchor in &line_anchors {
                let LeftOfAnchor::Named(frag) = fragment_before(line, anchor.col) else {
                    continue;
                };
                if !frag.to_ascii_lowercase().ends_with(".rs") {
                    continue;
                }
                if !is_a_claim_about_the_tree(line, anchor.col) {
                    data += 1;
                    continue;
                }
                // Zero or several matches mean the fragment names no one file, and this scan
                // refuses rather than guesses — the same rule the forward direction learned.
                //
                // ⚠️ **The refusal used to be a bare `continue` with no push, no cap and no
                // report**, while the `.md` direction pinned the identical class at zero in all
                // nine documents *with the rationale that an anchor naming no single file is not
                // checked at all*. The asymmetry was the finding: one direction called this its
                // most important ledger and the other dropped it silently. Split in two, because
                // the two verdicts are not the same defect — an AMBIGUOUS fragment is coverage the
                // scan declines to guess at, and a fragment matching NOTHING is a citation of a
                // file that is not in the tree, which is a dead path and the stronger claim.
                let Some(target) = resolve_unique_fragment(frag) else {
                    let hits = |f: &str| {
                        repo_files()
                            .iter()
                            .filter(|p| path_ends_with_fragment(p, f))
                            .count()
                    };
                    let n = hits(frag);
                    if n > 0 {
                        unbindable.push(format!(
                            "  {rel}:{}  `{frag}:{}` matches {n} files; not checked",
                            idx + 1,
                            anchor.line_no
                        ));
                        continue;
                    }
                    // A fragment matching nothing splits again, and the split is the difference
                    // between a file that is gone and a path that was mistyped. Both are dead as
                    // written; only the first is a file the tree no longer has.
                    let base = frag.rsplit('/').next().unwrap_or(frag);
                    let note = match hits(base) {
                        0 => format!("no file named `{base}` exists"),
                        k => format!(
                            "`{base}` exists ({k} in the tree) but not under this path — the \
                             fragment omits or mistypes a directory segment"
                        ),
                    };
                    dead.push(format!(
                        "  {rel}:{}  `{frag}:{}` matches no path in the tree: {note}",
                        idx + 1,
                        anchor.line_no
                    ));
                    continue;
                };
                found.push(MdCitation {
                    file: rel.clone(),
                    line: idx + 1,
                    raw: frag.to_string(),
                    target,
                    start: anchor.line_no,
                    end: anchor.range_end,
                });
            }
        }
    }
    (found, data, unbindable, dead)
}

/// `.rs:N` citations written inside `.rs` sources are bounds-checked, and the twelve that do not
/// hold are named rather than silenced.
///
/// ⚠️ **The twelve are ENUMERATED, not repaired.** Several live in `crates/boyko_ui/**`, which
/// another lane is editing as this lands, and one of the target files is being modified there;
/// repairing a line number into a file someone else is rewriting would be a repair with a shorter
/// half-life than the defect. They are filed for their owners in `docs/OPEN-QUESTIONS.md`.
///
/// [`RS_KNOWN_STALE`](rs_line_citations_written_inside_rust_sources_are_bounds_checked::RS_KNOWN_STALE)
/// cannot rot in either direction: an unlisted violation fails, and a listed entry that stops
/// reporting fails too. The second half is the one that matters here — a list of known defects with
/// no emptiness check is how a corpus keeps a repaired entry on its books forever.
///
/// The key is `(citing file, target fragment, start line)` and deliberately **not** the citing
/// line. The citing line is the coordinate that moves when the file around it is edited, and half
/// these entries sit in a crate another lane is rewriting; keying on it would turn every unrelated
/// insertion above them into a red. It is printed in the report, where a reader needs it.
#[test]
fn rs_line_citations_written_inside_rust_sources_are_bounds_checked() {
    /// The `.rs` → `.rs` citations that do not hold today, by `(citing file, fragment, start)`.
    ///
    /// All twelve are `past end of file`: three target files that all resolve uniquely and have all
    /// shrunk under the citation — `boyko_macros/src/lib.rs` (650 lines), `data.rs` (980) and
    /// `ecs_master.rs` (1918, the god-file the refactoring campaign split).
    const RS_KNOWN_STALE: &[(&str, &str, usize)] = &[
        ("crates/boyko_ecs/src/ecs/core/commands/migration_helpers.rs", "boyko_macros/src/lib.rs", 1062),
        ("crates/boyko_ecs/src/ecs/core/iters/query/chunked_data.rs", "data.rs", 1230),
        ("crates/boyko_ecs/src/ecs/core/iters/query/chunked_data.rs", "data.rs", 1383),
        ("crates/boyko_ecs/src/ecs/core/iters/query/chunked_data.rs", "data.rs", 1493),
        ("crates/boyko_ecs/src/ecs/core/iters/query/query_view.rs", "data.rs", 1155),
        ("crates/boyko_ecs/tests/phase13_local_systemparam.rs", "ecs_master.rs", 2548),
        ("crates/boyko_ui/src/layout.rs", "ecs_master.rs", 2096),
        ("crates/boyko_ui/src/layout.rs", "ecs_master.rs", 2121),
        ("crates/boyko_ui/src/reload/reconcile.rs", "ecs_master.rs", 2456),
        ("crates/boyko_ui/src/reload/reconcile.rs", "hierarchy/commands.rs", 268),
        ("crates/boyko_ui/src/reload/system.rs", "ecs_master.rs", 2456),
        ("crates/boyko_ui/src/text/lower.rs", "boyko_macros/src/lib.rs", 3875),
    ];

    /// Ceiling on `.rs` → `.rs` anchors whose fragment matches SEVERAL files and which are
    /// therefore not checked at all.
    ///
    /// ⚠️ **This direction had no such ledger, and the `.md` direction pins the identical class at
    /// zero in all nine documents.** The asymmetry was invisible from either side: the census
    /// printed *"240 … (5 skipped as string-literal data)"* and said nothing about the population
    /// that resolved to no single file. It is a CEILING rather than a pin at zero because the
    /// commonest member is a bare `mod.rs`, which is ambiguous in this tree by construction and
    /// which no repair can make unique; what the ceiling buys is that the ambiguous population
    /// cannot grow while the checked one shrinks.
    ///
    /// ⚠️ **176 -> 188 in the A6 merge, and the growth is the CORPUS, not a loosening.** This
    /// census scans every `.rs` in the tree, and it was authored on the reflection lane, whose
    /// tree is the merge base plus `boyko_reflect`. The merge brings the line's crates —
    /// `boyko_app`, `bench_bevy_vs_boyko`, the KE16 threadpool work — under the same scan for the
    /// first time, and 12 of their citations name an ambiguous basename (`vb.rs` matches 3 files,
    /// `runner.rs` 2, `device.rs` 4). Re-derived from the merged tree's own run, not adjusted.
    const UNBINDABLE_RS_MAX: usize = 188;

    /// Ceiling on `.rs` → `.rs` anchors naming a fragment that matches **nothing in the tree**.
    ///
    /// A different verdict from the one above and a stronger claim: an ambiguous fragment is
    /// coverage this scan declines to guess at, and a fragment matching zero files is a citation of
    /// a file that does not exist. The `.md` direction pins its dead paths at zero and this one
    /// counted none at all.
    ///
    /// The five live members are NAMED here rather than left as a number, because a bare ceiling
    /// over a dead-path population is exactly the shape that lets a sixth join in silence. Four
    /// name a basename that exists nowhere in the tree:
    ///
    /// * `component_registry.rs`, cited from `component/hooks/mod.rs` — the module was split into
    ///   a directory and the citation kept the pre-split file name;
    /// * `vb_bench_totality_gate.rs`, cited twice from
    ///   `crates/boyko_app/tests/vg_occ_split_timing.rs`;
    /// * `a7_probe.rs`, cited from `crates/aether_tests/tests/a7_dx.rs`.
    ///
    /// ⚠️ **The fifth is a different animal and the ledger says so rather than flattening it.**
    /// `crates/boyko_scene/tests/gpu3d_pack_miri_witness.rs` writes a two-segment fragment whose
    /// basename DOES exist — the file is under `boyko_render/src/`, and the fragment omits the
    /// `src` segment, so it ends no path in the tree. A count that called it "a file that is not
    /// in the tree" would have been wrong about the one member a reader is most likely to check,
    /// which is why the message distinguishes the two verdicts.
    ///
    /// They are ENUMERATED, not repaired, for the reason `RS_KNOWN_STALE` gives: repairing a name
    /// requires knowing which file the sentence meant, which is a question for each one's owner.
    ///
    /// ⚠️ **5 -> 12 in the A6 merge, and every one of the seven is attributable.** Measured by
    /// asking, per member, whether its citing line exists on the lane: the ORIGINAL five are
    /// exactly the five that do (`gpu3d_pack_miri_witness.rs`, `hooks/mod.rs`,
    /// `vg_occ_split_timing.rs` x2, `a7_dx.rs`), and all seven new ones are the line's
    /// `boyko_threadpool` work — two in files the lane does not have
    /// (`block_allocation_receipts.rs:64`, `block.rs:764`, both citing `std`'s own
    /// `alloc/windows.rs`, which is not in this tree by construction) and five citing
    /// `deque.rs`, i.e. `crossbeam-deque`, likewise a dependency rather than a tree file.
    /// They are ENUMERATED rather than repaired for the reason the paragraph above gives.
    const DEAD_RS_MAX: usize = 12;

    let (found, data, unbindable, dead) = rs_citations_in_rust_sources();
    let mut report = String::new();
    let mut matched = vec![false; RS_KNOWN_STALE.len()];

    println!(
        "{} `.rs:N` citation(s) bound on their own line inside .rs sources ({data} skipped as \
         string-literal data)",
        found.len()
    );
    println!(
        "{} anchor(s) named a fragment matching several files (cap {UNBINDABLE_RS_MAX}):",
        unbindable.len()
    );
    for u in &unbindable {
        println!("{u}");
    }
    println!(
        "{} anchor(s) named a fragment matching NO file in the tree (cap {DEAD_RS_MAX}):",
        dead.len()
    );
    for d in &dead {
        println!("{d}");
    }

    for c in &found {
        let Ok(text) = std::fs::read_to_string(&c.target) else {
            report.push_str(&format!(
                "  {}:{}  `{}:{}` — target is unreadable\n",
                c.file, c.line, c.raw, c.start
            ));
            continue;
        };
        let total = text.lines().count();
        let complaint = if c.start > total {
            Some(format!("is past end of file ({total} lines)"))
        } else if c.end.is_some_and(|e| e < c.start) {
            Some("ends before it starts".to_string())
        } else if c.end.is_some_and(|e| e > total) {
            Some(format!("ends past end of file ({total} lines)"))
        } else {
            None
        };
        let Some(complaint) = complaint else {
            continue;
        };
        let line = format!(
            "  {}:{}  `{}:{}{}` {complaint}",
            c.file,
            c.line,
            c.raw,
            c.start,
            c.end.map(|e| format!("-{e}")).unwrap_or_default()
        );
        println!("{line}");
        match RS_KNOWN_STALE
            .iter()
            .position(|&(f, t, s)| f == c.file && t == c.raw && s == c.start)
        {
            Some(i) => matched[i] = true,
            None => report.push_str(&format!("{line}\n")),
        }
    }

    for (i, &(f, t, s)) in RS_KNOWN_STALE.iter().enumerate() {
        if !matched[i] {
            report.push_str(&format!(
                "  {f}  `{t}:{s}` is listed in RS_KNOWN_STALE but no longer reports.\n\
                 \x20     Either it was repaired — delete the row in the same commit — or the \
                 scan stopped seeing it, which is the failure this half exists to catch.\n"
            ));
        }
    }

    // ⚠️ **The prefilter must be IMPLIED by the binder, and it was not.** `fragment_before` reads a
    // name through its delimiters — a backtick or a bold marker closing the name before the colon
    // opens — while the walk above skipped any line without a literal `.rs` followed by a colon on
    // it, which is precisely the substring such a delimiter destroys. (Written without an example
    // on purpose: this scan reads this file, and the first draft of this sentence spelled one out
    // and became a live dead-path citation of its own. MEASURED, twice now, in two directions.)
    // The two split forms were dropped before
    // the repaired binder ever ran, and the live population of both is ZERO, so nothing in the
    // corpus could have said so. The invariant is asserted here instead: every line shape this scan
    // is willing to BIND must be a line shape it is willing to READ.
    //
    // The four shapes are ASSEMBLED at run time rather than written as literals, and that is the
    // same lesson a third time: this scan reads this file, so a literal probe would enter the
    // population it is probing — inflating the published count by four and putting four synthetic
    // entries in a ledger whose whole value is that every entry is real.
    let name = "crates/boyko_ecs/src/lib.rs";
    for (delim, inside) in [("`", false), ("**", false), ("`", true), ("", false)] {
        let form = if inside {
            format!("the header is at {delim}{name}:1{delim} in the kernel")
        } else {
            format!("the header is at {delim}{name}{delim}:1 in the kernel")
        };
        let form = form.as_str();
        let (_, anchors) = scan_line(form, &docs_dir());
        assert_eq!(anchors.len(), 1, "the probe must present one anchor: {form}");
        let bound = matches!(
            fragment_before(form, anchors[0].col),
            LeftOfAnchor::Named(f) if f.to_ascii_lowercase().ends_with(".rs")
        );
        assert!(
            bound,
            "the binder must read a `.rs` name through its delimiters: {form}"
        );
        assert!(
            form.contains(".rs"),
            "…and the prefilter must admit every line the binder can read. A `.rs:` prefilter \
             fails this for the two delimited forms, which is how a citation 99 437 lines past \
             the end of a 562-line file exited 0: {form}"
        );
    }

    if unbindable.len() > UNBINDABLE_RS_MAX {
        report.push_str(&format!(
            "  {} anchor(s) name a fragment matching several files, cap {UNBINDABLE_RS_MAX}:\n{}\n",
            unbindable.len(),
            unbindable.join("\n")
        ));
    }
    if dead.len() > DEAD_RS_MAX {
        report.push_str(&format!(
            "  {} anchor(s) name a fragment matching NO file in the tree, cap {DEAD_RS_MAX}. A \
             citation of a file that is not in the tree is a dead path, not lost coverage:\n{}\n",
            dead.len(),
            dead.join("\n")
        ));
    }

    assert!(
        report.is_empty(),
        "a `.rs:N` citation written inside a Rust source does not hold.\n\
         This direction is bounds-checked only: a report means the cited file no longer has that \
         line. Re-measure with `grep -n` and repair the number, or — if it is a defect being left \
         for its owner — add it to `RS_KNOWN_STALE` with the reason.\n{report}"
    );
}

// ============================================================================================
// Claims the documents make about MEASURABLE properties of the tree
// ============================================================================================

/// Opening of a measurement marker: a claim about the tree, written beside the prose that states
/// it, in a form this file can re-derive.
///
/// # The class this exists for
///
/// ⚠️ **Everything above gates where a sentence POINTS. Nothing gated what a sentence SAYS about a
/// number, and the documents are full of numbers taken with a command.** The defect that forced
/// this: `REFLECTION-PLAN-ECS.md`'s ownership sweep wrote *"both OPEN-QUESTIONS files now match 28
/// lines each"* and the landing that wrote the sentence **added the 29th line itself**, under a
/// header from the same edit set. Both twins were 29 and both said 28.
///
/// Two properties make that worse than an off-by-one, and both are the reason for a marker rather
/// than a convention:
///
/// * **The EN/RU twin check passed straight over it.** Both files were 29; both claimed 28; the
///   twins agreed perfectly. **A parity check tests SAMENESS, never TRUTH** — see the rule's own
///   site, `docs/ru/README.md`, where that sentence now stands beside the sync rule it qualifies.
/// * **It was ungated by construction, deliberately.** An earlier round moved counts like this one
///   OUT of anchor shape, because `NAME (N)` was being read as a citation of line N. That repair
///   was right and it put every such figure beyond the reach of every gate in this file. The
///   un-gating was load-bearing, and it rotted inside the same landing.
///
/// # Why a marker, and not a command
///
/// The obvious shape is a comment carrying the shell command and a test that runs it. It was
/// rejected: a test that executes text out of a document makes the corpus an input to process
/// spawning, and the commands in these documents are `grep -rn … | wc -l` pipelines that are not
/// portable off a POSIX shell. So the vocabulary is CLOSED and executed in Rust — three kinds,
/// enumerated in [`Measurement`], each the exact shape of a claim the documents actually write.
/// Extending it is a code change with a review, which is the correct cost for widening what a
/// document is allowed to assert mechanically.
///
/// # Form
///
/// An HTML comment, so it is invisible in a rendered document and cannot be read as a citation by
/// any check in this file:
///
/// ```text
/// <!-- measure: KIND ARG… = N -->
/// ```
///
/// The token being counted is written with `%XX` hex escapes for any byte. That is not decoration:
/// a marker counting a token is usually written in a file the count covers, and an unescaped token
/// would make the marker perturb its own measurement — the same trap `UNBOUND_MAX`'s inventory
/// measured on itself. The escape is not optional and not a convention:
/// [`documents_that_count_the_tree_are_re_measured`] FAILS a marker whose own line contains the
/// decoded token, so a marker that would corrupt its own answer cannot be written at all.
const MEASURE_MARKER: &str = "<!-- measure:";

/// One re-derivable measurement, in the closed vocabulary [`MEASURE_MARKER`] admits.
enum Measurement {
    /// `lines-in FILE TOKEN` — lines of one file containing a literal token. The `grep -c` shape.
    LinesIn { file: PathBuf, token: String },
    /// `lines-in-digit FILE TOKEN` — lines of one file where the token is immediately followed by
    /// an ASCII digit. The `grep -c "NAME\.md:[0-9]"` shape, which is how these documents count
    /// their own citations; expressible without a regex engine because the only quantifier the
    /// corpus uses is "and then a number".
    LinesInDigit { file: PathBuf, token: String },
    /// `tree-lines DIR EXT TOKEN` — lines containing a literal token, over every file under `DIR`
    /// whose extension is `EXT`. `EXT` may be `*` for any file. The `grep -rn … | wc -l` shape.
    /// `target`, `.git` and `node_modules` are excluded, because [`repo_files`] excludes them.
    TreeLines {
        dir: PathBuf,
        ext: String,
        token: String,
    },
}

impl Measurement {
    /// Re-derive the figure from the tree as it is right now, or `None` if there was **nothing to
    /// read**.
    ///
    /// # `None` is not zero, and conflating them was a gate that could not fail
    ///
    /// ⚠️ **This returned a bare `usize`, and a bare count cannot distinguish "the token does not
    /// occur" from "the file/population does not exist".** Both came back `0`, and a marker
    /// asserting `= 0` was then certified `[ok]` against nothing at all. Two one-edit escapes were
    /// OBSERVED green over live false claims, both on 2026-08-28:
    ///
    /// * **Absent file.** `REFLECTION-PLAN-CORE.md`'s F17 marker was repointed from
    ///   `.github/workflows/ci.yml` to `.github/workflows/ci.yaml`, which does not exist. The
    ///   suite printed *"lines of .github/workflows/ci.yaml containing `hwrt` -> written 0,
    ///   re-measured 0  [ok]"* and exited **0**. The prose still claimed a `grep` over `ci.yml`.
    /// * **Absent population.** `REFLECTION-PLAN-ECS.md`'s F22 marker had its extension changed
    ///   from `rs` to `rss`, so the filter selected no files. The suite printed *"lines containing
    ///   `residency = "gpu"` under crates/\*.rss -> written 0, re-measured 0  [ok]"* and exited
    ///   **0**. A sum over an empty iterator is `0`, and `0` was the answer the marker wanted.
    ///
    /// Neither escape needed a second edit, and neither left a trace in the output beyond a path
    /// a reader would have to notice by eye — which is the same standard the marker exists to
    /// replace.
    ///
    /// # What each kind returns
    ///
    /// * `LinesIn` / `LinesInDigit` — `None` when the file cannot be read. A genuine measured zero
    ///   over a file that IS there is `Some(0)` and stays green.
    /// * `TreeLines` — `None` when the population is EMPTY, i.e. no file under `dir` has extension
    ///   `ext`. A non-empty population whose files contain the token nowhere is `Some(0)`. A file
    ///   inside a non-empty population that cannot be read is also `None` rather than a silent
    ///   nought, because a partial sum presented as a total is the same defect one directory down.
    ///
    /// # The genuine zeros this must not sweep up — **five**, enumerated
    ///
    /// Counted from the test's own stdout on 2026-08-28, because the number was got wrong twice
    /// while this paragraph was being written: three `lines-in` markers over files that exist —
    /// the `positional` sweep of `REFLECTION-PLAN-BOUNDARY.md` and the `hwrt` sweep of
    /// `.github/workflows/ci.yml`, written at `REFLECTION-PLAN-CORE.md:22` and
    /// `REFLECTION-PLAN-CORE.md:88`, and the campaign-name sweep of `REFLECTION-PLAN-CORE.md`
    /// written at `REFLECTION-PLAN-ECS.md:2764` — plus two `tree-lines` markers over the
    /// non-empty `crates` population, at `REFLECTION-PLAN-ECS.md:70` and
    /// `REFLECTION-PLAN-ECS.md:240`, for `residency = "gpu"` and `AddOutcome::Added`.
    /// All five are `Some(0)` and all five stay green.
    ///
    /// # ⚠️ Those five coordinates were written in a FORM no check in this tree can read, and one
    /// # of them was already stale when it shipped
    ///
    /// Until 2026-08-29 the third of them read *"written in `REFLECTION-PLAN-ECS.md` (line
    /// 2703)"* — the document named at the end of one line, the number parenthesised at the start
    /// of the next. The live coordinate was **2733**; the difference is 30, and both the marker and
    /// this sentence were written inside the SAME uncommitted landing, so the number was taken
    /// before a later insertion in that edit set and left in the present tense. That is the
    /// pre-state-as-live-reading shape this campaign has now recorded four times, and the census
    /// could not say so because **the citation was invisible twice over**:
    ///
    /// * [`scan_line`]'s parenthesised anchor form requires a DIGIT immediately after the `(` —
    ///   the byte scan at the `b'('` arm stops the moment it reads a non-digit — so the word in
    ///   front of the number is enough to make `(line 2703)` not an anchor at all. The same
    ///   parenthesis WITHOUT that word is the member-table form and IS an anchor: a bare `(7)`
    ///   after a document name is exactly what once made the ownership sweep's COUNTS parse as
    ///   citations of those documents' line 7, recorded at `REFLECTION-PLAN-ECS.md:2676`.
    /// * [`md_citations_in_rust_sources`] skips any line that does not itself contain `.md`, and
    ///   binds only to a name on the anchor's OWN line. So even spelled `(2703)`, a number on the
    ///   line below its document name is never reached — it does not even enter the `skipped`
    ///   ledger [`UNBOUND_MAX`](md_line_citations_written_inside_rust_sources_are_bounds_checked::UNBOUND_MAX)
    ///   bounds.
    ///
    /// **Two independent reasons pointing the same way is this corpus's own signature for a gate
    /// that cannot fail**, and it is why the repair is not "fix the digit". All five are now
    /// written in the `NAME.md:N` form, which [`md_citations_in_rust_sources`] binds and
    /// [`md_line_citations_written_inside_rust_sources_are_bounds_checked`] bounds-checks: five
    /// coordinates that were prose are now five entries in that test's printed inventory, and this
    /// file's row in `MD_CITATIONS_MIN` is a FLOOR, so adding them is admitted by construction.
    /// MEASURED: repointing one of the five past the end of the document it names reds at exit
    /// **101** with *"is past end of file"*, so the form is genuinely read and not merely parsed.
    ///
    /// ⚠️ **And that repair bought BOUNDS, not truth — measured on itself, in this same pass.**
    /// The third coordinate was rewritten as **2733**, and a later edit in this very edit set
    /// inserted 31 lines above it in `REFLECTION-PLAN-ECS.md`, carrying the marker to **2764**.
    /// **Nothing redded**, because that direction is bounds-only by an explicit design decision
    /// recorded at [`md_line_citations_written_inside_rust_sources_are_bounds_checked`]: a
    /// coordinate that moves WITHIN its file still lands inside it. Repaired by hand, again. The
    /// form is worth having — a bounds-checked citation that appears in a printed inventory is
    /// reachable by a reader and by a future check, and prose is neither — but **"now gated" is
    /// exactly the overclaim this paragraph exists to warn about**, and one pass of self-inflicted
    /// drift was enough to earn the correction.
    ///
    /// **The transferable half is about the INSTRUMENT, not the digit.** A sweep that certifies
    /// *"no cited coordinate moved"* by grepping for `NAME:N` is a statement about one citation
    /// FORM, not about citations. Every form this corpus has since had to learn — a bare `:N`
    /// inheriting the last filename named, a parenthesised COUNT read as a line, a document named
    /// one line above its number — was invisible to the sweep that had just certified the file.
    /// A coverage claim is only as wide as the shape it matches, and the shape is never stated.
    fn take(&self) -> Option<usize> {
        let count_lines = |path: &Path, f: &dyn Fn(&str) -> bool| {
            std::fs::read_to_string(path)
                .ok()
                .map(|t| t.lines().filter(|l| f(l)).count())
        };
        match self {
            Measurement::LinesIn { file, token } => count_lines(file, &|l| l.contains(token)),
            Measurement::LinesInDigit { file, token } => count_lines(file, &|l| {
                l.match_indices(token.as_str()).any(|(i, _)| {
                    l.as_bytes()
                        .get(i + token.len())
                        .is_some_and(u8::is_ascii_digit)
                })
            }),
            Measurement::TreeLines { dir, ext, token } => {
                let population: Vec<&PathBuf> = repo_files()
                    .iter()
                    .filter(|p| p.starts_with(dir))
                    .filter(|p| {
                        ext == "*"
                            || p.extension()
                                .and_then(|e| e.to_str())
                                .is_some_and(|e| e == ext)
                    })
                    .collect();
                if population.is_empty() {
                    // The escape: an empty population sums to zero, and zero is what a marker
                    // asserting absence wants to hear.
                    return None;
                }
                population
                    .iter()
                    .map(|p| count_lines(p, &|l| l.contains(token)))
                    .sum()
            }
        }
    }

    /// Is `path` inside the population this measurement counts over?
    ///
    /// The self-match guard needs this and an unconditional guard would be wrong: a marker in one
    /// document counting a token in ANOTHER cannot perturb its own answer, and forcing it to escape
    /// would make the prose unreadable for no gain. `REFLECTION-PLAN-CORE.md` states that the word
    /// *"positional"* is absent from `REFLECTION-PLAN-BOUNDARY.md`, and it has to be able to write
    /// that word to say so.
    fn covers(&self, path: &Path) -> bool {
        match self {
            Measurement::LinesIn { file, .. } | Measurement::LinesInDigit { file, .. } => {
                normalize(file) == normalize(path)
            }
            Measurement::TreeLines { dir, ext, .. } => {
                path.starts_with(dir)
                    && (ext == "*"
                        || path
                            .extension()
                            .and_then(|e| e.to_str())
                            .is_some_and(|e| e == ext))
            }
        }
    }

    /// How the marker named it, for the report.
    fn describe(&self) -> String {
        match self {
            Measurement::LinesIn { file, token } => {
                format!("lines of {} containing `{token}`", show(file))
            }
            Measurement::LinesInDigit { file, token } => {
                format!("lines of {} where `{token}` is followed by a digit", show(file))
            }
            Measurement::TreeLines { dir, ext, token } => {
                format!("lines containing `{token}` under {}/*.{ext}", show(dir))
            }
        }
    }
}

/// Remove every `<!-- measure: … -->` span from a line.
///
/// Applied to the marker's own line **and to its neighbours** before the proximity check reads
/// them. Neighbours matter as much as the line itself: two markers written next to each other would
/// otherwise satisfy the check with each other's `= N`, and the check would pass over a pair of
/// markers with no prose number anywhere near them.
fn strip_measure_markers(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(o) = rest.find(MEASURE_MARKER) {
        out.push_str(&rest[..o]);
        let tail = &rest[o..];
        match tail.find("-->") {
            Some(e) => rest = &tail[e + 3..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Does `hay` write `n` as a standalone number — not as a run of digits inside a longer one?
///
/// The marker carries the figure the gate checks, and the PROSE carries the figure a reader
/// believes. Nothing makes those the same string, so this asks whether the number the marker
/// asserts is actually written where a reader will see it. Without it a marker could be updated
/// while the sentence beside it kept the old digit, which is the whole defect wearing a gate.
///
/// # ⚠️ Exactly what the proximity guard establishes, and what it does NOT
///
/// Stated here rather than left to be assumed, because the guard is the only thing standing
/// between the marker and the sentence, and "the number is NEAR the marker" is not "the number is
/// ABOUT this claim".
///
/// **It establishes**, over the marker's own line and its two immediate neighbours, with every
/// `<!-- measure: … -->` span stripped from all three first (so a marker cannot satisfy the check
/// with its own `= N`, nor with the `= N` of a marker written beside it):
///
/// * the decimal digits of the re-derived figure appear somewhere in that three-line window, and
/// * they appear as a whole number — `12` is not satisfied by `120` or by `312`.
///
/// **It does NOT establish any of the following, and none of them is checked anywhere:**
///
/// * **That the number is about this claim.** No association is made between the figure and the
///   subject of the sentence. A line that writes the same integer for an unrelated reason — a
///   section number, a year, a count of something else, a line number in a nearby citation —
///   satisfies the guard completely.
/// * **That the number is in the SENTENCE.** The window is three lines and the prose the marker
///   annotates may be one of them or none of them; a figure on the line above, belonging to the
///   paragraph above, counts.
/// * **That the measurement's TOKEN or POPULATION matches what the prose describes.** The marker
///   says which file and which token it counted; the sentence says which command a reader should
///   run. Nothing compares them, and they can and do differ in scope — one live marker counts
///   every `.rs` file under `crates/` while the command its prose prints is narrower. ⚠️ This
///   bullet is **not** where that limit belongs and it is no longer only here: it is the RUNG's
///   limit, and it is now stated at [`documents_that_count_the_tree_are_re_measured`], where a
///   reader asking what a green means will actually look. Stated in a helper's doc comment it was
///   found by an eighth adversarial pass and by nothing before it.
/// * **That only ONE number is near.** A window containing several integers is satisfied if any
///   of them matches, so a stale prose figure sitting beside a correct one passes.
///
/// The consequence worth naming: this guard makes a silently-updated marker impossible, and it
/// makes a mis-attributed figure no harder than it already was. It is a check on VISIBILITY, not
/// on REFERENCE.
fn writes_number(hay: &str, n: usize) -> bool {
    let want = n.to_string();
    hay.match_indices(&want).any(|(i, _)| {
        let before = i == 0 || !hay.as_bytes()[i - 1].is_ascii_digit();
        let after = hay
            .as_bytes()
            .get(i + want.len())
            .is_none_or(|b| !b.is_ascii_digit());
        before && after
    })
}

/// Repo-relative, `/`-separated rendering of a path, for report lines a reader has to act on.
fn show(p: &Path) -> String {
    p.strip_prefix(repo_root())
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Decode the `%XX` escapes a marker's token is written with. An unpaired or non-hex `%` is left
/// alone rather than rejected: the escape exists to break self-matching, and a `%` that is simply
/// part of a token must survive.
fn decode_token(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Some(hi) = (b[i + 1] as char).to_digit(16)
            && let Some(lo) = (b[i + 2] as char).to_digit(16)
        {
            out.push((hi * 16 + lo) as u8 as char);
            i += 3;
            continue;
        }
        out.push(b[i] as char);
        i += 1;
    }
    out
}

/// One marker as written, parsed.
struct MeasuredClaim {
    /// Repo-relative path of the document carrying the marker.
    doc: String,
    /// Line the marker is written on.
    line: usize,
    what: Measurement,
    /// The figure the prose beside it states.
    written: usize,
    /// The token after `%XX` decoding, for the self-match guard.
    token: String,
}

/// Every `.md` file under `docs/`, including `docs/ru/` and the two `OPEN-QUESTIONS.md` twins.
///
/// ⚠️ Deliberately NOT [`GATED_DOCS`]. That list is nine documents and it excludes both
/// `OPEN-QUESTIONS.md` files, which is exactly where this campaign records its measurements — a
/// marker mechanism scoped to `GATED_DOCS` would have been blind to the twins whose disagreement
/// with their own prose is the defect that produced it.
fn measured_docs() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = repo_files()
        .iter()
        .filter(|p| p.starts_with(docs_dir()))
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("md"))
        .cloned()
        .collect();
    out.sort();
    out
}

/// Parse every measurement marker in the documents, and the markers that did not parse.
fn measured_claims() -> (Vec<MeasuredClaim>, Vec<String>) {
    let root = repo_root();
    let mut out = Vec::new();
    let mut bad = Vec::new();

    for path in measured_docs() {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if !text.contains(MEASURE_MARKER) {
            continue;
        }
        let doc = show(&path);
        // Every marker on the line, not the first: two markers side by side is the natural way to
        // gate a claim about a pair of files, and a `find`-once parser would have read one of them
        // and silently dropped the other — a scanner with a blind spot gating a corpus for blind
        // spots. MEASURED on the EN/RU twin pair, which is exactly that shape.
        for (idx, line) in text.lines().enumerate() {
            let mut cursor = 0usize;
            while let Some(rel_open) = line[cursor..].find(MEASURE_MARKER) {
                let open = cursor + rel_open;
                let rest = &line[open + MEASURE_MARKER.len()..];
                let Some(close) = rest.find("-->") else {
                    bad.push(format!("  {doc}:{}  marker is never closed", idx + 1));
                    break;
                };
                cursor = open + MEASURE_MARKER.len() + close + 3;
                let body: Vec<&str> = rest[..close].split_whitespace().collect();
                // Every form ends `= N`, and the kind fixes how many arguments precede it.
                let Some((&n, head)) = body.split_last() else {
                    bad.push(format!("  {doc}:{}  marker is empty", idx + 1));
                    continue;
                };
                let Ok(written) = n.parse::<usize>() else {
                    bad.push(format!(
                        "  {doc}:{}  `{n}` is not a number; every marker ends `= N`",
                        idx + 1
                    ));
                    continue;
                };
                let (what, token) = match head {
                    [kind @ ("lines-in" | "lines-in-digit"), file, tok, "="] => {
                        let token = decode_token(tok);
                        let file = root.join(file);
                        (
                            if *kind == "lines-in" {
                                Measurement::LinesIn {
                                    file,
                                    token: token.clone(),
                                }
                            } else {
                                Measurement::LinesInDigit {
                                    file,
                                    token: token.clone(),
                                }
                            },
                            token,
                        )
                    }
                    ["tree-lines", dir, ext, tok, "="] => {
                        let token = decode_token(tok);
                        (
                            Measurement::TreeLines {
                                dir: root.join(dir),
                                ext: (*ext).to_string(),
                                token: token.clone(),
                            },
                            token,
                        )
                    }
                    _ => {
                        bad.push(format!(
                            "  {doc}:{}  unknown measurement `{}`; the vocabulary is `lines-in \
                             FILE TOKEN`, `lines-in-digit FILE TOKEN`, `tree-lines DIR EXT \
                             TOKEN`, each followed by `= N`",
                            idx + 1,
                            head.join(" ")
                        ));
                        continue;
                    }
                };
                out.push(MeasuredClaim {
                    doc: doc.clone(),
                    line: idx + 1,
                    what,
                    written,
                    token,
                });
            }
        }
    }
    (out, bad)
}

/// A figure a document states about the tree is RE-TAKEN here and compared to what the prose says.
///
/// See [`MEASURE_MARKER`] for the class and for why the vocabulary is closed rather than a shell
/// command. This test is the half that makes a marker worth writing: without it a marker is a
/// comment, and a comment beside a number is what every one of these documents already had.
///
/// # What this covers, and what it does not
///
/// Gated: every figure carrying a marker, listed in `MEASURED_EXACT` below — the FIGURE by
/// re-derivation, the marker COUNT per document by `MEASURED_EXACT`, and WHAT each marker measures
/// by `MEASURED_SUBJECTS`.
///
/// ⚠️ **THE RUNG'S OWN LIMIT, stated here rather than in a helper's doc comment, because this is
/// where a reader comes to learn what a green means.** Nothing compares a marker to the COMMAND
/// the prose prints beside it. The marker says which file and which token were counted; the
/// sentence tells a reader which command to run; the two are written independently and **they
/// differ in the live corpus** — the F22 row prints a `grep` over one glob while its marker counts
/// a broader population, and both answer nought today. So **a reader re-running the printed
/// command is not re-running the gate**, and a green certifies the marker's measurement, never the
/// sentence's.
///
/// ⚠️ **That gap was previously described as "fail-closed because the marker's population contains
/// the command's", and that framing is wrong — it is a property of today's marker TEXT, not of
/// this gate.** MEASURED 2026-08-28: narrowing a `tree-lines crates rs` marker to a single
/// sub-crate reverses the containment in one token, and before `MEASURED_SUBJECTS` landed nothing
/// redded. `MEASURED_SUBJECTS` now fixes the marker's subject, so the containment cannot be
/// silently reversed — but that is a pin, not a comparison, and the direction of any future
/// mismatch remains whatever someone writes into both lists.
///
/// Making it a comparison was measured and is not available at this revision. Of the **16** markers
/// in the corpus, **5** carry a machine-readable command inside the SAME three-line window the
/// proximity guard reads — the `#[ignore` row and the `REFLECTION-PLAN-CORE.md` self-count row
/// of the EN twin (the RU twin is frozen and carries neither), the F17 row of
/// `REFLECTION-PLAN-CORE.md`, and the F22 and
/// `AddOutcome::Added` rows of `REFLECTION-PLAN-ECS.md` — written as coordinates nowhere here
/// because a citation in this file is CENSUSED by [`md_citations_in_rust_sources`] and a sentence
/// about a marker would become one. Even over those five a parser would not be a gate: F22 writes
/// its population in prose (*"over `crates/`"*) rather than in the command, and the `#[ignore`
/// row prints a `wc -l` pipeline this vocabulary has no shape for. **7** markers sit on lines whose
/// entire content is markers, with the prose on a neighbouring line. A parse over that corpus would
/// be a heuristic wearing a gate's certificate, which is the shape this campaign exists to find.
///
/// ⚠️ **Two of those three figures read *"3"* and *"8"* until 2026-08-29, and both were wrong in
/// the direction that spares the work.** The `3` was taken by naming the markers a reader had
/// happened to notice rather than by scanning the window the guard actually uses — the live answer
/// is **7**, more than twice it, and the sentence went on to reason from `3` down to *"two markers
/// in eighteen"*. The `8` was never re-taken at all. Neither is a miscount of a hard thing: both
/// are one loop over [`measured_claims`], and neither had one. All three now do — they are
/// re-derived on every run by [`figures_this_file_writes_about_the_tree_are_re_derived`], which is
/// the only reason this paragraph is allowed to state them in the present tense.
///
/// **NOT gated, and enumerated so the gap is a decision rather than an assumption:**
///
/// * **Figures taken against a named commit** — `git show f7c46c76 … | grep -c`. A claim about a
///   frozen object cannot rot, and re-deriving it would require a `git` process. These are the
///   right shape already: the stamp is the gate.
/// * **Figures with a date and a round attached** — *"re-measured at that round"*, *"measured at
///   round 5"*. These are claims about a past state and are honest as written. ⚠️ They are still a
///   hazard when the sentence draws a PRESENT-tense conclusion from a past number, and the corpus
///   has one: `REFLECTION-PLAN-ECS.md`'s gate-18 paragraph stamps *"measured at round 5"* on a
///   figure of **two** and then reasons from it in the present tense. It is left as written because
///   the stamp makes it true; it is named here because the next reader should not mistake a stamped
///   figure for a current one.
///
///   ⚠️ **This bullet used to give the live answer as **nine**, and that was the THIRD simultaneous
///   present-tense answer the same sweep carried in one tree** — the question log's limit section
///   said one number, its own later paragraph said another, and this line said a third. Nine was
///   additionally the ECS-only count, a NARROWER population than the command the claim prints. The
///   live figure is now taken and DATED in the question log and is deliberately not restated here:
///   an alternation is the one shape the marker vocabulary cannot carry, this sentence is not one of
///   the lines it counts but the ones it describes are, and a number no check re-derives should
///   exist in exactly one place.
/// * **Regex-shaped counts** — the census of `#[ignore` reason prefixes needs a character class,
///   and this vocabulary deliberately has no regex engine. Adding one is a bigger decision than
///   this rung. Alternation is the same gap by another spelling: the `llvm-nm` filter and the
///   `g18`-or-`gate 18` sweep both join two literals, and a marker carries exactly one.
/// * **Claims that are not counts** — *"every one of the fifteen OBSERVED"*, *"both twins are
///   byte-identical"*. A different mechanism; not this one.
/// * **A file's LENGTH** — `wc -l`. Inexpressible, and not by oversight: every kind here counts
///   lines CONTAINING a token, and the empty token that would count them all cannot be written,
///   because the parser splits a marker on whitespace and an empty field simply is not there.
///   Live in the corpus: **910** and **3283** for the two `seam_by_id` files, **1917** for
///   `ecs_master.rs`, **4026** for `REFLECTION-PLAN-CORE.md`. Inexpressible as a marker, but no
///   longer ungated: all four are re-derived from the tree on every run by
///   [`figures_this_file_writes_about_the_tree_are_re_derived`], and the last two are load-bearing
///   — one is the `str::lines()`-versus-`wc -l` discrepancy of a file with no final newline, the
///   other is what made a repair line-count-neutral.
///
///   ⚠️ **The second figure said 2572 while the tree said 3054, and the row was written by the
///   same uncommitted landing that grew that file — carrying the words *"All four re-measure
///   correctly"*.** That is the pre-state-as-live-reading shape this very block warns about
///   two bullets earlier, wearing the one phrase in the paragraph that claims it had been checked.
///   A sentence asserting its own freshness is not evidence of it, and the repair is not the digit:
///   it is that a figure inexpressible in the marker vocabulary is not thereby exempt from being
///   re-derived, only from being re-derived by THAT mechanism.
///
///   ⚠️ **It then moved AGAIN, one pass later, and that is the first thing here worth calling a
///   result.** The record pass that rewrote the twins' limit also appended to the by-id seam test,
///   taking it from 3054 to 3074 — the identical shape, in the identical file, by the identical
///   mechanism (a figure written about the edit set it lives inside). It did not ship. The gate
///   above printed *"the tree says 3074 and the sentence stating it does not write that number"*
///   and the pass re-took the figure. **Two occurrences, one prose repair and one red: that
///   difference is the entire value of the gate, and it is why the class is recorded as CONTAINED
///   rather than as closed.** Nothing here stops the figure from moving; it stops it from moving
///   quietly, which is the only property a count in a comment can actually be given.
/// * **Counts scoped to a function BODY or a document SECTION** — the unit of every kind here is a
///   whole file or a whole directory. *"`mark_arch_present` is called zero times over both
///   bodies"* and *"the two occurrences of `ten gates` inside D26's own narration"* are true and
///   un-re-derivable, and the second is self-referential besides: a raw file-wide `grep` returns
///   **3** and **2** for those two tokens precisely because the sentence counting them writes them.
/// * **Subjects that are not text in the tree** — the harness's own `--list` output joined against
///   a document (*"N of the 25 test names appear in this file verbatim"*), and
///   `llvm-nm --defined-only` over a linked binary (*"six defined symbols"*). There is nothing to
///   re-read: one needs a `cargo test` process, the other a release build and a symbol table.
/// * **The census's OWN figures, quoted back into the documents** — 375 bound `.rs` citations, 188
///   unbindable, 12 dead, 179 formerly dropped, 12 known-stale. (242/174/5 until the A6 merge
///   brought the line's crates under the same scan, then 331 until the A6 follow-up added four
///   citations — two into `migration_helpers` and two into the archetype module — each written
///   with enough path to BIND: the bare file name of the second matches two files in the tree
///   and would have landed in the unbindable ledger, where nothing checks it. Then 335 until the
///   A7 merge: the UI campaign's sources brought 26 bound citations and 3 ambiguous plus 3 dead
///   fragments, and the merge's repair bound those six by writing enough path — every cited line
///   read in the union first — so both ledgers are back at 188 and 12. Then 367 until the A8
///   merge (the `integ/unified` cut): `feat/multi-paradigm-render`'s two near-twin examples,
///   `boyko_app/examples/{playground,_hud_probe}.rs`, brought 4 bound citations and 4 ambiguous
///   fragments (`runner.rs`, `bindless.rs`, twice each), and the merge bound those four by
///   writing enough path, each line re-derived on the merged tree first (the runner's
///   default-material pin had moved 69 lines, the bindless sampler's range 23) — 188 and 12
///   again. The figures
///   that moved are re-derived from the run, and `179 formerly dropped` is left as the
///   historical figure it always was.) ⚠️ The first read **240** until
///   2026-08-29, two behind a tree the same landing had moved, and it is the one figure in this
///   bullet a marker genuinely cannot reach — re-deriving it means running the census, so it is
///   now compared against [`rs_citations_in_rust_sources`] itself by
///   [`figures_this_file_writes_about_the_tree_are_re_derived`] rather than against a re-scan.
///   These are gated where they are
///   produced, by the caps and by `RS_KNOWN_STALE`, and the documents carry them inside a fenced
///   transcript of a named run. A marker over them would re-derive the census from inside the
///   census. ⚠️ Note the residual: a cap with headroom (`UNBINDABLE_RS_MAX` is 176 over a live
///   174) gates the ledger but NOT the document's copy of the number.
/// * **SELF-REFERENTIAL claims — ungatable by this vocabulary, and the corpus holds two.** A
///   sentence asserting that a token is absent must write the token to say so, and it thereby
///   makes itself false. `REFLECTION-PLAN-CORE.md`'s D14 paragraph says a `grep` for a derive
///   `attributes` list *"returns zero hits"* over the four plans and there are **7** today, its own
///   line among them; `OPEN-QUESTIONS.md`'s G22b entry says a symbol has *"zero hits in `crates/`"*
///   and there are **2**, both inside a test's module header that quotes the finding — including a
///   sentence reading *"MEASURED: zero hits anywhere under `crates/`"* which is false because it
///   exists. Neither is an engineering error: the symbol still does not exist and no rung lands the
///   attribute. **They are COUNT errors created by writing the count**, which is the same trap the
///   `%XX` escape closes for a marker, and which no `%XX` can close for prose. The honest repair is
///   to state the property (*"no rung lands it"*) rather than a grep total, and both are left as
///   they stand because rewriting another round's ruling is not this rung's call.
///
/// ⚠️ **The enumeration above was audited against the landing that wrote it, and the audit found
/// the class INSIDE it, twice.** Both are in `REFLECTION-PLAN-ECS.md` and both are repaired.
///
/// * The gate-naming paragraph said *"**2 of 25** appear — MEASURED 2026-08-28"* and then, one
///   sentence later, *"the three extra legs … take that to 5"* — with the legs already written.
///   The live answer is **5**. A figure taken before its own edit set finished, left in the
///   present tense, stamped with the date of the landing that falsified it: the identical shape as
///   the three name sweeps that same landing struck. It is ungated for a reason now listed above
///   (its subject needs a `cargo test` process), which is exactly why the prose had to be right.
/// * The ownership sweep said *"re-derived on every run since"* of **six** figures while carrying
///   markers for **two**. That is a claim about THIS TEST'S REACH, made by the paragraph that had
///   just gated the other two, and it was false of four. The four now carry markers and are
///   written as digits — the word *"unchanged"* was the defect, because a word gives
///   [`writes_number`] nothing to compare and hides a figure from the proximity guard entirely.
///
/// The lesson is not "check harder". It is that **a paragraph explaining a gate is the single most
/// likely place to state an ungated number**, because writing about the mechanism reads like using
/// it.
///
/// ⚠️ **And one gap that is NOT about figures at all, recorded here because this round hit it three
/// times.** Both `OPEN-QUESTIONS.md` twins cite THIS file by line — **10 in the EN twin and 0 in
/// the RU one, 10 between them** at this revision (9/0/9 until the A7 merge brought the UI
/// campaign's "not anchor-gated" bullet; it read 8/8/16 before the A6 merge, which
/// froze the RU twin on the line's side) — and **no check in
/// this tree reads them.** `GATED_DOCS` excludes both twins, so the forward `.md` → `.rs` direction
/// never sees them; [`md_citations_in_rust_sources`] runs the other way and filters to `.md`
/// besides. Every edit to this file that moves a line silently rots those 10, and this round
/// repaired them by hand three times because nothing would have said
/// so. The fix is a scope decision, not a code one — adding a 260 KB question log to `GATED_DOCS`
/// arms every check in this file over it at once — so it is named rather than taken.
///
/// ⚠️ **What IS now taken is the COUNT, which is a different claim and was rotting on its own.**
/// The 8-and-16 above is a count of the tree stated in the present tense, and this paragraph has
/// already been wrong about it once (the *"twelve … six per twin"* correction below). It is
/// re-derived on every run by [`figures_this_file_writes_about_the_tree_are_re_derived`]. Note
/// exactly what that buys and what it does not: the gate says **how many** citations the twins
/// make, never **whether any of them still lands on the line it names**. The second is the
/// scope decision above and remains untaken — so a round that moves a line in this file still rots
/// those 16 silently, and now does so with a green count standing beside them.
///
/// ⚠️ **Until 2026-08-28 this paragraph read *"twelve citations between them … six per twin"*, and
/// that is a claim about THIS TEST'S REACH standing four lines under the paragraph that names
/// exactly that failure mode.** Re-taken with
/// `grep -c "internal_docs_anchors[.]rs:[0-9]" docs/OPEN-QUESTIONS.md` and the same over the
/// `docs/ru/` twin: **8 and 8**. It went stale in the ordinary way — the round that recorded four
/// coordinates in the twins wrote two new citations per twin in the same edit set and did not
/// re-take the count — and the twelve/six was never wrong about anything except how many there are.
///
/// ⚠️ **The sentence that stood here said the bracket classes were what kept the count at 8, and
/// that "the unescaped form answers 9 over its own text". Both halves are FALSE, and the second is
/// a live figure no form of the command produces.** Measured 2026-08-29 over the shipped twins,
/// three forms each, six counts:
///
/// ```text
/// grep -c "internal_docs_anchors[.]rs:[0-9]" docs/OPEN-QUESTIONS.md         -> 8
/// grep -c "internal_docs_anchors.rs:[0-9]"   docs/OPEN-QUESTIONS.md         -> 8
/// grep -cF "internal_docs_anchors.rs:"       docs/OPEN-QUESTIONS.md         -> 8
/// ```
///
/// and **8, 8, 8** again over `docs/ru/OPEN-QUESTIONS.md`. The unescaped regex **cannot** answer 9,
/// in principle and not merely today: the line it would have to match writes the pattern, so what
/// follows `rs:` in it is the opening bracket of a character class, and a character class does not
/// match itself. The `[.]` buys nothing here at all.
///
/// The **9** is real, and it belongs to a THIRD form that neither sentence named — a fixed-string
/// search with the trailing digit class DROPPED. Reconstructed by copying each twin and rewriting
/// its quoted command unescaped, then counting: the unescaped regex still answers **8**, and only
/// `grep -cF "internal_docs_anchors.rs:"` answers **9**, on both twins. **What self-matches is a
/// command whose pattern is a PREFIX of its own text; what does not is one whose pattern continues
/// past the point its own text stops resembling it.** That is the property to reason from, and the
/// struck sentence had attached it to the wrong token.
///
/// **The correction cannot be gated where it is written.** [`measured_docs`] admits `.md` files
/// under `docs/` and nothing else, so a figure in a `.rs` doc comment carries no marker by
/// construction; this one is re-derivable only by running the two commands above. All 8 were
/// re-resolved by hand at this revision — 311, 691, 1697, 1707, 2880, 3669, 4055 and 4061, each
/// read and each landing on the line the twin says it does.
#[test]
fn documents_that_count_the_tree_are_re_measured() {
    /// Exact number of measurement markers per document.
    ///
    /// Exact, not a floor, for the reason `PLANNED_EXACT` and `SILENCED_EXACT` are exact: a marker
    /// that stops being written is a claim that went back to being ungated, and a ceiling cannot
    /// tell that from a claim that was deleted. Adding a marker is a deliberate act and moves this
    /// list in the same edit.
    const MEASURED_EXACT: &[(&str, usize)] = &[
        ("docs/OPEN-QUESTIONS.md", 2),
        ("docs/REFLECTION-PLAN-CORE.md", 2),
        ("docs/REFLECTION-PLAN-ECS.md", 12),
        // 2 -> 0 in the A6 merge. `docs/ru/**` is FROZEN (owner, 2026-09-07) and the line's
        // side of it won this merge verbatim; the frozen text carries neither marker, so the
        // two rows this list and `MEASURED_SUBJECTS` held for it are REMOVED rather than
        // re-aimed. A gated figure becoming ungated prose ON PURPOSE, recorded here because
        // the panic message rightly refuses to tell that from an accidental deletion.
    ];

    /// Exact inventory of WHAT each marker measures — its file or its directory-and-extension,
    /// and its token — independent of the figure it asserts. One row per marker, rendered by
    /// [`Measurement::describe`] so the pin and the report cannot describe the same marker two
    /// different ways. Deliberately keyed on the document and not on the line: a marker's line
    /// number moves whenever anything above it is edited, and pinning it would make every
    /// unrelated paragraph a false red.
    ///
    /// ⚠️ **Without this, the measurement gate could not tell a marker pointed at the RIGHT file
    /// from one pointed at a DIFFERENT EXISTING file, nor the population the prose describes from
    /// a narrower one.** [`Measurement::take`] closed the *unreadable* subcase — an absent file or
    /// an empty population now returns `None` instead of a nought — and that is the whole of what
    /// it closed. MEASURED 2026-08-28, two one-token edits, each leaving the suite at **exit 0**
    /// with the marker's own prose unchanged and still printing the original subject:
    ///
    /// * **Readable wrong file.** `REFLECTION-PLAN-CORE.md`'s F17 marker repointed from
    ///   `.github/workflows/ci.yml` to a different workflow file that also exists and also
    ///   contains the token nowhere. Printed `re-measured 0  [ok]`.
    /// * **Readable narrower population.** A `tree-lines crates rs` marker narrowed to a single
    ///   sub-crate. Same green, over a population the sentence beside it does not describe.
    ///
    /// Neither is reachable through this list: both change the rendered subject, and a changed
    /// subject with no matching row is a failure. The cost is that a legitimate re-aim of a marker
    /// must move this list in the same edit — which is the point, and the same bargain
    /// `MEASURED_EXACT` above already strikes for the marker COUNT. What it buys is that the
    /// re-aim appears in the diff instead of only in a file path a reader would have to notice by
    /// eye, which is the standard the marker exists to replace.
    ///
    /// ⚠️ **What it does NOT establish**: that the subject matches the command the prose prints.
    /// See the rung's own limit above — that comparison was measured and is not available, because
    /// only three of the sixteen markers have a machine-readable command beside them at all.
    const MEASURED_SUBJECTS: &[(&str, &str)] = &[
        (
            "docs/OPEN-QUESTIONS.md",
            "lines containing `#[ignore` under crates/*.rs",
        ),
        (
            "docs/OPEN-QUESTIONS.md",
            "lines of docs/REFLECTION-PLAN-CORE.md where `REFLECTION-PLAN-CORE.md:` is followed \
             by a digit",
        ),
        (
            "docs/REFLECTION-PLAN-CORE.md",
            "lines of docs/REFLECTION-PLAN-BOUNDARY.md containing `positional`",
        ),
        (
            "docs/REFLECTION-PLAN-CORE.md",
            "lines of .github/workflows/ci.yml containing `hwrt`",
        ),
        (
            "docs/REFLECTION-PLAN-ECS.md",
            "lines containing `residency = \"gpu\"` under crates/*.rs",
        ),
        (
            "docs/REFLECTION-PLAN-ECS.md",
            "lines containing `AddOutcome::Added` under crates/*.rs",
        ),
        (
            "docs/REFLECTION-PLAN-ECS.md",
            "lines containing `.dense_contains(` under crates/*.rs",
        ),
        (
            "docs/REFLECTION-PLAN-ECS.md",
            "lines of docs/REFLECTION-PLAN-BOUNDARY.md containing `EG2`",
        ),
        (
            "docs/REFLECTION-PLAN-ECS.md",
            "lines of docs/REFLECTION-PLAN-GATES.md containing `EG2`",
        ),
        (
            "docs/REFLECTION-PLAN-ECS.md",
            "lines of docs/REFLECTION-ANALYSIS.md containing `EG2`",
        ),
        (
            "docs/REFLECTION-PLAN-ECS.md",
            "lines of docs/REFLECTION-PLAN-CORE.md containing `EG2`",
        ),
        (
            "docs/REFLECTION-PLAN-ECS.md",
            "lines of docs/OPEN-QUESTIONS.md containing `EG2`",
        ),
        (
            "docs/REFLECTION-PLAN-ECS.md",
            "lines of docs/ru/OPEN-QUESTIONS.md containing `EG2`",
        ),
        (
            "docs/REFLECTION-PLAN-ECS.md",
            "lines containing `AddOutcome` under crates/*.rs",
        ),
        (
            "docs/REFLECTION-PLAN-ECS.md",
            "lines containing `RejectReason` under crates/*.rs",
        ),
        (
            "docs/REFLECTION-PLAN-ECS.md",
            "lines containing `migrate_entity_attach_ids_with_bytes` under crates/*.rs",
        ),
        // The two `docs/ru/OPEN-QUESTIONS.md` rows are gone with the freeze - see
        // `MEASURED_EXACT` above for the ruling and its cost.
    ];

    let (claims, bad) = measured_claims();
    let mut report = String::new();

    println!("{} measurement marker(s) across docs/:", claims.len());
    for c in &claims {
        // `None` is "there was nothing to read", NOT "the answer is nought" — see
        // `Measurement::take` for the two escapes that distinction closes.
        let took = c.what.take();
        let shown = took.map_or("ABSENT".to_string(), |n| n.to_string());
        let verdict = match took {
            None => "NO POPULATION",
            Some(n) if n == c.written => "ok",
            Some(_) => "STALE",
        };
        println!(
            "  {}:{}  {} -> written {}, re-measured {shown}  [{verdict}]",
            c.doc,
            c.line,
            c.what.describe(),
            c.written
        );
        match took {
            None => report.push_str(&format!(
                "  {}:{}  {} — there is NOTHING TO MEASURE: the file is unreadable, or no file \
                 matches the directory and extension. This is not the figure {} being confirmed; \
                 it is the measurement never happening. Fix the path in the marker, or delete the \
                 marker and stop claiming the figure is gated.\n",
                c.doc,
                c.line,
                c.what.describe(),
                c.written
            )),
            Some(n) if n != c.written => report.push_str(&format!(
                "  {}:{}  the prose says {} and the tree says {n} — {}\n",
                c.doc,
                c.line,
                c.written,
                c.what.describe()
            )),
            Some(_) => {}
        }
    }
    for b in &bad {
        println!("{b}");
    }
    if !bad.is_empty() {
        report.push_str(&format!("  marker(s) that did not parse:\n{}\n", bad.join("\n")));
    }

    for c in &claims {
        let Ok(text) = std::fs::read_to_string(repo_root().join(&c.doc)) else {
            continue;
        };
        let lines: Vec<&str> = text.lines().collect();
        let own = lines.get(c.line - 1).copied().unwrap_or("");
        // The self-match guard. A marker counting a token, written on a line that CONTAINS that
        // token and that is INSIDE the population being counted, silently adds one to its own
        // answer — and a reader running the same command by hand gets the marker's number rather
        // than the corpus's. Escape it with `%XX`. See `Measurement::covers` for why the guard is
        // conditional rather than unconditional.
        if own.contains(&c.token) && c.what.covers(&repo_root().join(&c.doc)) {
            report.push_str(&format!(
                "  {}:{}  the marker's own line contains `{}` undecoded, so the marker is inside \
                 the population it measures. Escape a character with `%XX`.\n",
                c.doc, c.line, c.token
            ));
        }
        // The proximity guard. See `writes_number`: a gate over the marker alone leaves the prose
        // free to keep the stale digit, which is the defect with a certificate attached.
        let near = [
            strip_measure_markers(own),
            strip_measure_markers(lines.get(c.line.wrapping_sub(2)).copied().unwrap_or("")),
            strip_measure_markers(lines.get(c.line).copied().unwrap_or("")),
        ];
        if !near.iter().any(|l| writes_number(l, c.written)) {
            report.push_str(&format!(
                "  {}:{}  the marker asserts {} and no line within one of it writes that number. \
                 The figure must be readable in the prose, or the gate certifies a sentence \
                 nobody can check by eye.\n",
                c.doc, c.line, c.written
            ));
        }
    }

    let mut per_doc: BTreeMap<&str, usize> = BTreeMap::new();
    for c in &claims {
        *per_doc.entry(c.doc.as_str()).or_default() += 1;
    }
    for (doc, n) in &per_doc {
        match MEASURED_EXACT.iter().find(|(d, _)| d == doc) {
            Some((_, want)) if want == n => {}
            Some((_, want)) => report.push_str(&format!(
                "  {doc}: {n} marker(s), pinned {want}\n"
            )),
            None => report.push_str(&format!(
                "  {doc}: {n} marker(s) and no row in MEASURED_EXACT\n"
            )),
        }
    }
    // The subject inventory. Compared as a sorted multiset rather than positionally, so a marker
    // that moves within its document — or between two markers on one line — is not a red.
    let mut subjects: Vec<(&str, String)> = claims
        .iter()
        .map(|c| (c.doc.as_str(), c.what.describe()))
        .collect();
    subjects.sort();
    let mut pinned: Vec<(&str, String)> = MEASURED_SUBJECTS
        .iter()
        .map(|(d, s)| (*d, (*s).to_string()))
        .collect();
    pinned.sort();
    for s in &subjects {
        if !pinned.contains(s) {
            report.push_str(&format!(
                "  {}: a marker measures `{}`, which no row of MEASURED_SUBJECTS pins. A marker \
                 that changes WHAT it counts changes what the sentence beside it is certified to \
                 mean, and this list is the only thing that says so — the figure check cannot, \
                 because a different file and a different population answer the same number.\n",
                s.0, s.1
            ));
        }
    }
    for p in &pinned {
        if !subjects.contains(p) {
            report.push_str(&format!(
                "  {}: MEASURED_SUBJECTS pins `{}` and no marker measures it. Either the marker \
                 was re-aimed — say so here in the same edit — or a gated figure went back to \
                 being ungated.\n",
                p.0, p.1
            ));
        }
    }

    for (doc, want) in MEASURED_EXACT {
        if !per_doc.contains_key(doc) {
            report.push_str(&format!(
                "  {doc}: 0 marker(s) found, pinned {want} — a gated figure went back to being \
                 ungated prose\n"
            ));
        }
    }

    // The discriminator, asserted directly rather than left to the corpus. A pin over a corpus that
    // happens to agree is a gate that cannot fail, and this file has committed that twice.
    let probe = Measurement::LinesIn {
        file: repo_root().join("docs/ru/README.md"),
        token: "OPEN-QUESTIONS.md".to_string(),
    };
    let live = probe.take();
    assert!(
        live.is_some_and(|n| n > 0),
        "the re-measurement must actually read the tree; `lines-in` returned {live:?}"
    );
    assert_eq!(
        Measurement::LinesIn {
            file: repo_root().join("docs/ru/README.md"),
            token: "a token no document contains anywhere at all".to_string(),
        }
        .take(),
        Some(0),
        "a token absent from a file that EXISTS must measure `Some(0)`, not fall back to something"
    );
    // The absent/empty discriminator, asserted directly rather than inferred from a green corpus.
    // Both shapes below returned a bare `0` before `take` became fallible, and both were OBSERVED
    // certifying a live false claim — see `Measurement::take`. `Some(0)` above and `None` here is
    // the entire property, so the two assertions are written next to each other on purpose.
    assert_eq!(
        Measurement::LinesIn {
            file: repo_root().join("docs/this-file-does-not-exist.md"),
            token: "anything".to_string(),
        }
        .take(),
        None,
        "an ABSENT FILE must be `None`; a bare 0 here is a marker certified against nothing"
    );
    assert_eq!(
        Measurement::TreeLines {
            dir: repo_root().join("crates"),
            ext: "no-such-extension".to_string(),
            token: "anything".to_string(),
        }
        .take(),
        None,
        "an EMPTY POPULATION must be `None`; summing no files yields the 0 an absence claim wants"
    );
    assert_eq!(
        Measurement::TreeLines {
            dir: repo_root().join("crates"),
            ext: "rs".to_string(),
            token: "a token no source file contains anywhere at all".to_string(),
        }
        .take(),
        Some(0),
        "a NON-EMPTY population containing the token nowhere must stay `Some(0)` and stay green — \
         five live markers legitimately measure nought and must not be swept up by the fix"
    );
    assert_eq!(decode_token("EG%32"), "EG2", "`%XX` decoding is what keeps a marker out of its own population");

    assert!(
        report.is_empty(),
        "a document states a figure about the tree that the tree does not support.\n\
         This is the class the anchor gates above could never see: they check where a sentence \
         POINTS, never what it SAYS. Re-take the figure and write the new one — and if the old one \
         was true at a named commit, stamp it with that commit instead of stating it in the \
         present tense.\n{report}"
    );
}

// ============================================================================================
// Late definitions — placed here so no cited line moves
// ============================================================================================
//
// ⚠️ **Everything below is defined at the END of the file on purpose, and the purpose is
// mechanical.** Six documents and this file itself cite `tests/internal_docs_anchors.rs` by LINE —
// twenty-nine citations at twelve distinct numbers, running from 311 to 4061. An insertion anywhere
// above one of them silently renumbers it. Four of the twelve are cited from `GATED_DOCS` documents
// and one is this file quoting itself, so those five RED; the remaining seven appear ONLY in the two
// `OPEN-QUESTIONS.md` twins, which **no check in this tree reads**, so those rot in silence. That
// asymmetry is exactly how the round before this one moved a `REFLECTION-PLAN-ECS.md` quotation by
// fourteen lines and shipped the stale number in both twins. Adding a helper below the highest cited
// line costs nothing; adding one above it costs a repair pass in seven files.
//
// ⚠️ This paragraph itself read *"Four documents … Two of the twelve … those three RED"* when it was
// written, on 2026-08-28, and all three of those are wrong in the same direction — they understate
// the blast radius of an insertion. Re-taken by listing every occurrence of this file's name
// followed by a colon and a digit across `--include=*.md --include=*.rs`: twenty-nine occurrences,
// twelve distinct numbers, in `REFLECTION-PLAN-BOUNDARY.md`, `REFLECTION-PLAN-CORE.md`,
// `REFLECTION-PLAN-ECS.md`, `REFLECTION-PLAN-GATES.md`, both twins and here. **A reach-claim written
// beside a mechanism is the most likely sentence in the file to be false**, and this is the second
// one this file has carried — see the note above `documents_that_count_the_tree_are_re_measured`.
//
// Note the second constraint on anything written here: this file's own prose is CENSUSED by
// `md_citations_in_rust_sources` and `rs_citations_in_rust_sources`, so a comment that spells a
// file name followed by a colon and a number does not describe a citation — it CREATES one, and
// it lands against a cap. Every reference below therefore writes the number as a word.

/// The refusal a fenced margin note earns when the file it names cannot be resolved.
///
/// # The hole this closes
///
/// ⚠️ **The fenced arm's [`LeftOfAnchor::Named`] branch had NO `else`.** When
/// [`resolve_fragment`] returned `None` the branch simply did nothing, and control fell through
/// to `fence_target` — still holding the PREVIOUS margin note's file. So a citation naming a file
/// that does not exist was checked against a file the document never named at that citation, and
/// if the line happened to satisfy the pseudo-declaration (the overwhelmingly common case, since
/// the previous note is usually the same file) the suite stayed at **exit 0** with the citation
/// reported in no ledger at all.
///
/// This is a third uncovered path, distinct from the two the `Continuation` arm's comment
/// records: that one was a bare continuation inheriting a target, this one is a NAMED fragment
/// naming nothing. MEASURED 2026-08-28 in `docs/SYSTEMS.md`, on the fenced margin note at line
/// 385, whose file name was rewritten to `no_such_file_xyz.rs` with its line number untouched:
/// the suite reported **19 passed, exit 0**, and the string `no_such_file_xyz` appeared nowhere
/// in its output — not as a path violation, not as an unbindable, not as a stale anchor. With
/// this refusal in place the same edit reds at `unbindable_fragments_are_reported_and_pinned`
/// with *"1 anchor(s) skipped for an unresolvable fragment (cap 0)"*.
///
/// The non-fenced arm has refused this since [`resolve_unique_fragment`] landed, and its doc
/// states the rule the fenced arm now also obeys: an unresolvable fragment must cost COVERAGE,
/// never CORRECTNESS. The two arms resolve differently on purpose — outside a fence a fragment
/// must be unique tree-wide, inside one it walks up from the section's file list — but the
/// disposition of a failure is the same, and now it is spelled the same.
fn fenced_unbindable(doc: &str, lineno: usize, frag: &str, anchor_line: usize) -> String {
    format!("  {doc}:{lineno}  `{frag}:{anchor_line}` resolves to no file from the fence's base")
}

/// Resolve a fenced margin note's fragment: base-relative first, then unique tree-wide.
///
/// # Why the second half exists, and why refusing without it would have been wrong
///
/// [`resolve_fragment`] walks UP from the section's file list, which is the right rule for the
/// shape it was written for — `` system/system.rs `` inside a fence under a
/// `**Files:** [core/system/]` heading. It has one structural failure mode: when the section
/// names no `crates/…` path at all, `base.or(sticky)` is `None` and the walk returns `None`
/// before it starts. A fence under a prose heading is exactly that case.
///
/// ⚠️ **MEASURED 2026-08-28, and this is why the refusal alone was not the whole fix.** Landing
/// [`fenced_unbindable`] against the bare base-relative resolver redded **five live citations**:
/// three in `docs/REFLECTION-ANALYSIS.md`, at lines 1251, 1253 and 1563, naming
/// `ecs_master/component_api.rs`, `enable_tag_api.rs` and `boyko_macros/src/bindable.rs`; two in
/// `docs/REFLECTION-PLAN-ECS.md`, at lines 490 and 517, both naming `component_api.rs`. Every one
/// of those five names a file that EXISTS and that ends **exactly one** path in the tree. They
/// were not unbindable; they were unbindable *by that resolver*, and before the refusal landed
/// all five were silently checked against the previous margin note's file instead.
///
/// Calling them `unbindable` and raising the cap to absorb them would have written five
/// permanently open slots into a ledger whose whole purpose is that it has none — the headroom
/// hazard this file already carries once, in `UNBINDABLE_RS_MAX`. So the fragment is resolved the
/// way the NON-fenced arm resolves one it cannot match against the sticky binding:
/// [`resolve_unique_fragment`], zero-or-ambiguous ⇒ `None`. Nothing is guessed by either half —
/// the base-relative walk requires the file to be on disk, and the tree-wide half requires the
/// suffix to land on a path separator and to match exactly one file.
///
/// The refusal is what is left over: a fragment that neither the section's own file tree nor the
/// whole repository can bind is a citation of a file that is not there, and it now costs the
/// anchor rather than binding it to a neighbour.
fn fenced_resolve(frag: &str, from: Option<&PathBuf>, base: Option<&PathBuf>) -> Option<PathBuf> {
    resolve_fragment(frag, from, base).or_else(|| resolve_unique_fragment(frag))
}

/// The fenced **Named-unresolvable** refusal, asserted on documents built here.
///
/// # Why the corpus cannot prove this, and why waiting for it would be waiting forever
///
/// [`fenced_unbindable`] has exactly one ledger — [`unbindable_fragments_are_reported_and_pinned`]
/// — and that ledger's live population is **empty in all nine documents**: nine cap-zero rows over
/// nothing at all. MEASURED 2026-08-28: restoring the pre-fix semantics line-count-neutrally (the
/// `else` deleted, control falling through with `fence_target` still holding the PREVIOUS margin
/// note's file) leaves the suite at **nineteen passed, exit 0**. The fix shipped with no way to
/// observe its own regression, which is the same shape the fix's own commentary calls a hole.
///
/// A cap of zero over an empty set is a gate that cannot fail, and this file has now committed it
/// four times. The other three closed it with a direct assertion rather than with a wait:
/// [`the_binding_and_extraction_rules_are_asserted_directly`],
/// [`continuations_do_not_inherit_a_document_across_a_line_boundary`] and
/// [`doc_to_doc_anchors_whose_words_are_at_their_own_numbers_are_reported`]. This is the fourth,
/// and [`every_capped_ledger_says_whether_its_population_is_live_or_empty`] now fails the build if
/// a fifth appears without one.
///
/// The deferral that could have justified waiting does not apply either. Both halves of the fix sit
/// in this late-definitions block, below the highest line at which anything cites this file, so a
/// discriminator written beside them renumbers nothing — which is precisely what the banner above
/// this block was written to buy.
///
/// # What the fixtures separate
///
/// [`resolve_unique_fragment`] returns `None` for two different facts and the refusal cannot tell
/// them apart, so both are asserted. **Missing** — the fragment ends no path in the tree — is a
/// citation of a file that is not there. **Ambiguous** — it ends several — is coverage the binder
/// declines to guess at. Either way the anchor was not checked, so either way it must be reported.
///
/// The third and fourth fixtures are the shape the hole actually shipped as: an unresolvable note
/// FOLLOWING a good one inside the same fence. Pre-fix, `fence_target` still held the good one, so
/// the bogus citation was bounds-checked against a file the document never named at that citation.
/// When the number happened to be in bounds — the overwhelmingly common case, since a fence's notes
/// usually name the same file — nothing was reported anywhere; when it did not, the verdict was
/// reported against the wrong file, which is the stronger defect and the one that is visible
/// without counting. Both directions are pinned: an unresolvable fragment must cost COVERAGE, never
/// CORRECTNESS.
#[test]
fn a_fenced_note_naming_no_single_file_is_refused_rather_than_rebound() {
    // Assembled rather than written out. A file name followed by a colon and a number in this
    // file's own source is not a description of a citation, it IS one, and it lands against the
    // caps of the two censuses that read this tree's sources. Same reason the fence fixtures in
    // `continuations_do_not_inherit_a_document_across_a_line_boundary` are assembled.
    let d = '.';
    // Waived with a tilde so the neighbour's own citation asserts BOUNDS only: these fixtures are
    // about which file an anchor is bound to, and a definition-shape verdict on line one of some
    // real file would be a second, unrelated reason for them to move.
    let good = format!("`migration_helpers{d}rs:1~`");

    // (1) MISSING — the fragment ends no path in the tree, and the fence has nothing else in it.
    let missing = scan_text(
        "SYNTHETIC.md",
        &format!("```\n`no_such_file_xyz{d}rs:1~` is this fence's only margin note.\n```"),
    );
    assert_eq!(
        missing.unbindable.len(),
        1,
        "a fenced margin note naming a file that is not in the tree must be REFUSED and reported; \
         got {:?}",
        missing.unbindable
    );
    assert!(
        missing.unbindable[0].contains("no_such_file_xyz"),
        "the refusal must name the fragment it could not bind, or the ledger cannot be read: {:?}",
        missing.unbindable
    );
    assert!(
        missing.anchor_violations.is_empty() && missing.anchors == 0,
        "an unresolvable fragment costs COVERAGE, never CORRECTNESS — the anchor is neither \
         checked nor counted as checked. violations {:?}, anchors {}",
        missing.anchor_violations,
        missing.anchors
    );

    // (2) AMBIGUOUS — the fragment ends dozens of paths, so it names no ONE file either. The
    // number is deliberately not written down here: what is pinned is the verdict, not the count.
    let ambiguous = scan_text(
        "SYNTHETIC.md",
        &format!("```\n`mod{d}rs:1~` ends many paths in this tree and so names none of them.\n```"),
    );
    assert_eq!(
        ambiguous.unbindable.len(),
        1,
        "an AMBIGUOUS fenced fragment is refused exactly as a missing one is, and lands in the \
         same ledger — the anchor was not checked either way; got {:?}",
        ambiguous.unbindable
    );

    // (3) The silent half of the hole: the bogus note follows a good one, with a number the good
    // one can hold. Pre-fix this reported NOTHING — not a path violation, not an unbindable, not a
    // stale anchor — and the count of checked anchors quietly included one that was checked
    // against a file the document never named at that citation.
    let rebound = scan_text(
        "SYNTHETIC.md",
        &format!("```\n{good} binds this fence.\n`no_such_file_xyz{d}rs:1~` must not inherit it.\n```"),
    );
    assert_eq!(
        rebound.unbindable.len(),
        1,
        "a fence's binding must not be inherited by a note that names a file of its own and fails \
         to resolve it; got {:?}",
        rebound.unbindable
    );
    assert_eq!(
        rebound.anchors, 1,
        "only the note that BOUND the fence may be counted as checked; the refused one must not \
         be, or the coverage figure counts an anchor nobody checked"
    );

    // (4) The loud half: the same shape with a number the neighbour cannot hold. Pre-fix this
    // reported `past end of file` against a file the citation never named — a verdict about the
    // wrong file, which is worse than no verdict.
    let misattributed = scan_text(
        "SYNTHETIC.md",
        &format!("```\n{good} binds this fence.\n`no_such_file_xyz{d}rs:999999~` must not use it.\n```"),
    );
    assert_eq!(misattributed.unbindable.len(), 1, "the refusal is the same refusal");
    assert!(
        !misattributed
            .anchor_violations
            .iter()
            .any(|v| v.contains("past end of file")),
        "an out-of-bounds verdict was reported for an unresolvable fragment, which means it was \
         measured against the PREVIOUS margin note's file: {:?}",
        misattributed.anchor_violations
    );

    // (5) The control, and the half of the fix the live corpus does cover: a fragment that binds
    // tree-wide through `resolve_unique_fragment` must still bind. Five live citations depend on
    // it; asserting it here says so where the refusal is asserted, instead of leaving the two
    // halves of one function gated in two different places.
    let bound = scan_text("SYNTHETIC.md", &format!("```\n{good} is on disk and unique.\n```"));
    assert!(
        bound.unbindable.is_empty() && bound.anchors == 1,
        "a fenced fragment ending exactly one path in the tree must bind and be checked, not \
         refused: {:?} / anchors {}",
        bound.unbindable,
        bound.anchors
    );

    // (6) THE STICKY-FALLBACK ROUTE — the one the five fixtures above cannot reach.
    //
    // Every one of them opens its fence on line 1. So `current` is `None`, `fence_base` is `None`,
    // `resolve_fragment` returns before its walk starts, and ONLY `resolve_unique_fragment` is ever
    // exercised. A fallback placed AFTER that arm is inert in all five. It is inert in the live
    // corpus too — this ledger's population is empty in all nine documents — so nothing anywhere
    // observes it, which is the same cap-over-nothing shape the discriminator was written to close.
    //
    // ⚠️ **MEASURED 2026-08-29, and this is the hole the discriminator still had.** Appending
    // `.or_else(|| from.cloned())` to `fenced_resolve` — the single clause that reinstates falling
    // back to the section's binding, which this file's own commentary calls what produced the
    // misbindings — left the suite at **21 passed, exit 0**. REPLACING the unique-fragment arm with
    // it does red, at fixture (5) and at the five live citations that arm carries, so the edit that
    // survives is precisely the one that keeps both arms and adds a third.
    //
    // The prose mention on the first line is what makes the route reachable: it is the sticky
    // binding the fence is seeded from. The bare `:1~` inside the fence is the NON-VACUITY control
    // — it can only be counted if that seeding happened, so `anchors == 1` distinguishes "the
    // refused note was not counted" from "this document never bound anything and proves nothing".
    let under_prose = |note: &str| {
        format!(
            "`crates/boyko_ecs/src/ecs/core/commands/migration_helpers{d}rs` binds this section.\n\
             ```\n\
             `:1~` inherits the fence's seed, so the sticky binding is live in this fixture.\n\
             {note}\n\
             ```"
        )
    };

    // (6a) The silent half: the bogus note carries a number the section's file CAN hold. Pre-fix
    // this reported nothing at all, and the coverage figure counted an anchor that had been checked
    // against a file the citation never named.
    let sticky_silent = scan_text(
        "SYNTHETIC.md",
        &under_prose(&format!(
            "`no_such_file_xyz{d}rs:1~` must not fall back to the section's binding."
        )),
    );
    assert_eq!(
        sticky_silent.unbindable.len(),
        1,
        "a fenced note naming a file of its own must be REFUSED when it resolves to nothing, \
         INCLUDING when the fence sits under a prose file mention — that is the one route on which \
         a sticky fallback is reachable, and the five fence-on-line-1 fixtures cannot see it; \
         got {:?}",
        sticky_silent.unbindable
    );
    assert_eq!(
        sticky_silent.anchors, 1,
        "only the seeded `:N` continuation may be counted. 2 means the refused note was rebound to \
         the section's file and counted as checked; 0 means the prose mention never bound and this \
         fixture is vacuous"
    );

    // (6b) The loud half: the same shape with a number the section's file cannot hold. A fallback
    // here does not merely over-count coverage, it publishes a verdict about the wrong file.
    let sticky_loud = scan_text(
        "SYNTHETIC.md",
        &under_prose(&format!(
            "`no_such_file_xyz{d}rs:999999~` must not be measured against it."
        )),
    );
    assert_eq!(
        sticky_loud.unbindable.len(),
        1,
        "the refusal is the same refusal on the sticky route; got {:?}",
        sticky_loud.unbindable
    );
    assert_eq!(
        sticky_loud.anchors, 1,
        "the seeded continuation is the only anchor that may be counted here — see (6a)"
    );
    assert!(
        !sticky_loud
            .anchor_violations
            .iter()
            .any(|v| v.contains("past end of file")),
        "an out-of-bounds verdict was reported for an unresolvable fragment, which means it was \
         measured against the SECTION's file — a verdict about a file the citation never named, \
         which is the stronger defect of the two: {:?}",
        sticky_loud.anchor_violations
    );
}

/// Which capped ledgers in this file have a live population, and which are caps over nothing.
///
/// # The class, rather than one row of it
///
/// MEASURED 2026-08-29 over this revision: this file publishes **84 cap rows** — nine
/// per-document ledgers across the nine gated documents, plus the three scalar ceilings the two
/// source-census gates own — and **70 of them are 0 over an empty live set**. That is not by itself
/// a defect: a zero row in a table whose OTHER rows are live means "this document is clean", and
/// the mechanism behind it is exercised by the documents that are not. The defect is narrower and
/// this gate is scoped to it — a ledger whose population is empty in EVERY document, where the cap
/// is the only thing standing between a green and a mechanism that no longer runs at all.
///
/// ⚠️ **These three numbers read 66 / 54 / 12 over "seven per-document ledgers" one revision ago,
/// and the arithmetic was self-consistent — 7×9+3 = 66 — which is exactly why it survived a
/// reading.** It was not a miscount, it was a SHORT SUBJECT LIST: `planned_paths` and
/// `stale_planned` are capped per document, by this file, and neither was enumerated. Completing
/// the inventory turned the second of them into an immediate RED — a capped ledger empty in all
/// nine documents with no discriminator, which is the precise condition this gate fails the build
/// for. **The instrument written to find caps over nothing was blind to one, because a subject
/// list is itself a claim and nothing checked it.** The figures are recomputed and PRINTED on every
/// run from the same `PER_DOC` table the assertion walks, so the prose above can go stale but the
/// output cannot; a reader who needs the current numbers reads the run, not this paragraph.
///
/// Four ledgers are in that state today, and all four now carry a direct discriminator. The list
/// below is the promise; this test is what makes breaking it cost a red rather than a reading. A
/// ledger that drains to empty — or a new capped ledger that starts empty — fails here until
/// someone either names the test that discriminates it or writes one.
///
/// ⚠️ **What this gate does NOT claim.** It does not check that the named test is any good, only
/// that it was named. And it says nothing about the **fourteen** live rows — the figure read
/// *"twelve"* until 2026-08-29, carried over from the 66/54/12 era while the two numbers beside it
/// in the paragraph above were being corrected to 84 and 70, which is the ordinary way a
/// three-number sentence goes half-stale. Their greens are backed by a
/// population and their real hazard is the opposite one — headroom, which
/// [`unbindable_fragments_are_reported_and_pinned`] and its siblings pin at the live count for
/// exactly that reason.
#[test]
fn every_capped_ledger_says_whether_its_population_is_live_or_empty() {
    /// A capped ledger whose live population is empty in every gated document, and the test that
    /// discriminates its mechanism directly on a document built for the purpose.
    const EMPTY_WITH_DISCRIMINATOR: &[(&str, &str)] = &[
        (
            "cross_line_doc",
            "continuations_do_not_inherit_a_document_across_a_line_boundary",
        ),
        (
            "doc_misbound",
            "doc_to_doc_anchors_whose_words_are_at_their_own_numbers_are_reported",
        ),
        (
            "unbindable",
            "a_fenced_note_naming_no_single_file_is_refused_rather_than_rebound",
        ),
        (
            "stale_planned",
            "a_planned_marker_over_paths_that_all_exist_is_reported_as_waiving_nothing",
        ),
    ];

    /// How one ledger's population is read off a scan. Named rather than written inline because
    /// the inline form is what `clippy::type_complexity` rejects, and an `#[allow]` here would be
    /// the third way this file makes a check disappear for the sake of one line.
    type LedgerCount = fn(&DocScan) -> usize;

    /// Every per-document ledger this file caps, read straight off the scan so the census cannot
    /// drift from the ledgers it describes.
    const PER_DOC: &[(&str, LedgerCount)] = &[
        ("planned_paths", |s| s.planned_paths.len()),
        ("stale_planned", |s| s.stale_planned.len()),
        ("alias_bound", |s| s.alias_bound.len()),
        ("ignored_lines", |s| s.ignored_lines.len()),
        ("cross_line_doc", |s| s.cross_line_doc.len()),
        ("doc_misbound", |s| s.doc_misbound.len()),
        ("doc_unquoted", |s| s.doc_unquoted.len()),
        ("unbindable", |s| s.unbindable.len()),
        ("over_waived", |s| s.over_waived.len()),
    ];

    let scans = scan_all();
    let docs = scans.len();
    let mut rows = 0usize;
    let mut empty_rows = 0usize;
    let mut report = String::new();

    for (name, get) in PER_DOC {
        let total: usize = scans.values().map(get).sum();
        rows += docs;
        empty_rows += scans.values().filter(|s| get(s) == 0).count();
        if total > 0 {
            let live: Vec<String> = scans
                .iter()
                .filter(|(_, s)| get(s) > 0)
                .map(|(d, s)| format!("{d}={}", get(s)))
                .collect();
            println!(
                "ledger `{name}`: LIVE — {total} across {} of {docs} rows ({})",
                live.len(),
                live.join(", ")
            );
            continue;
        }
        match EMPTY_WITH_DISCRIMINATOR.iter().find(|(l, _)| l == name) {
            Some((_, t)) => println!(
                "ledger `{name}`: all {docs} rows are caps of 0 over an EMPTY live set — the \
                 mechanism is discriminated directly by `{t}`"
            ),
            None => report.push_str(&format!(
                "  `{name}`: {docs} cap rows, live population EMPTY in every document, and no \
                 direct discriminator is named for it\n"
            )),
        }
    }

    // The three scalar ceilings. They are read from the same functions their own gates read, so a
    // ledger cannot be live here and empty there.
    let (_, md_unbound) = md_citations_in_rust_sources();
    let (_, _, rs_unbindable, rs_dead) = rs_citations_in_rust_sources();
    for (name, n) in [
        ("md unbound", md_unbound.len()),
        ("rs unbindable", rs_unbindable.len()),
        ("rs dead", rs_dead.len()),
    ] {
        rows += 1;
        if n == 0 {
            empty_rows += 1;
            report.push_str(&format!(
                "  `{name}`: a scalar cap of 0 over an EMPTY live set, with no discriminator\n"
            ));
        } else {
            println!("ledger `{name}`: LIVE — {n}");
        }
    }

    // ⚠️ **The discriminator column was a STRING, and nothing resolved it.** The loop above matches
    // the LEDGER half against `PER_DOC` and then prints the test half verbatim, without ever asking
    // whether that test exists or runs. Renaming or deleting a discriminator would leave this gate
    // printing *"the mechanism is discriminated directly by `X`"* over nothing at all — a claim
    // about coverage backed by a string literal, which is the same cap-over-nothing shape this very
    // test was written to close, one level up and inside the instrument that closes it.
    //
    // Resolved against this file's own source. The needle is BUILT at runtime rather than written:
    // spelled out, it would occur in the inventory above and the check would resolve against its
    // own list instead of against a definition — the self-matching trap this file carries elsewhere.
    let own_source = std::fs::read_to_string(repo_root().join("tests/internal_docs_anchors.rs"))
        .expect("invariant: this test's own source must be readable from the manifest directory");
    let own_lines: Vec<&str> = own_source.lines().collect();
    for (ledger, disc) in EMPTY_WITH_DISCRIMINATOR {
        let needle = format!("fn {disc}(");
        match own_lines.iter().position(|l| l.trim_start().starts_with(&needle)) {
            // A definition is not enough. A discriminator that does not RUN discriminates nothing,
            // and an ordinary helper function is exactly what a deleted `#[test]` leaves behind.
            Some(at) => {
                if !own_lines[at.saturating_sub(4)..at]
                    .iter()
                    .any(|l| l.trim() == "#[test]")
                {
                    report.push_str(&format!(
                        "  `{ledger}`: its discriminator `{disc}` is defined at line {} but \
                         carries no `#[test]` above it, so it never runs\n",
                        at + 1
                    ));
                }
            }
            None => report.push_str(&format!(
                "  `{ledger}`: names `{disc}` as its discriminator, and no such test is defined in \
                 this file\n"
            )),
        }
        if !PER_DOC.iter().any(|(n, _)| n == ledger) {
            report.push_str(&format!(
                "  `{ledger}`: carries a discriminator but is not a ledger this file caps; the \
                 inventory has outlived the row it describes\n"
            ));
        }
    }

    println!(
        "{rows} cap row(s) in this file; {empty_rows} sit at 0 over an empty live set, {} over a \
         live one",
        rows - empty_rows
    );

    assert!(
        report.is_empty(),
        "a capped ledger has no live population in ANY document and nothing asserts its mechanism \
         directly.\n\
         Its green says only that nothing was measured. Every other gate in this file would keep \
         passing if the code behind this one stopped running — which is how the fenced arm's \
         missing refusal shipped, and how the two before it did. Write the discriminator on a \
         document built in the test (`scan_text` takes text, not a path) and name it in \
         `EMPTY_WITH_DISCRIMINATOR`; do not raise anything, and do not wait for the corpus to grow \
         an instance.\n{report}"
    );
}

/// The **marker-waives-nothing** half of the planned-path contract, asserted on documents built here.
///
/// # Why this exists, and what completing the census turned up
///
/// [`DocScan::stale_planned`] is a capped ledger whose live population is **empty in all nine
/// documents**, and until this revision it was not in the census's inventory at all. The inventory
/// listed seven per-document ledgers; the file caps **nine**. Adding the two missing rows —
/// `planned_paths`, which is LIVE (four mentions across two documents), and this one, which is not
/// — made [`every_capped_ledger_says_whether_its_population_is_live_or_empty`] fire immediately:
/// *"`stale_planned`: 9 cap rows, live population EMPTY in every document, and no direct
/// discriminator is named for it"*.
///
/// ⚠️ **So the gate was not wrong; its subject list was short, and a short subject list is the same
/// green as a broken check.** That is the cap-over-nothing shape one level up — the instrument built
/// to find caps over nothing was itself blind to a cap over nothing, because the row was never
/// enumerated. The fix is the enumeration plus this discriminator, not a raise.
///
/// # What is pinned
///
/// The ledger's rule is an `all`, not an `any`, and that is the half a careless discriminator would
/// miss. A marker over paths that ALL exist waives nothing and is reported; a marker over a line
/// that still has one missing path is doing its job and must stay silent, because reporting it would
/// push documents to strip markers that are still load-bearing. Both directions are asserted, plus
/// the no-marker control that keeps the fixture from passing for the wrong reason.
#[test]
fn a_planned_marker_over_paths_that_all_exist_is_reported_as_waiving_nothing() {
    // Assembled for the same reason the fenced fixtures are: a path written whole in this file's
    // source is scanned by the two source censuses, and a fixture must not add rows to the ledgers
    // it is testing. `here` is on disk; `gone` is well-formed and is not.
    let d = '.';
    let dir = "crates/boyko_ecs/src/ecs/core/commands";
    let here = format!("{dir}/migration_helpers{d}rs");
    let gone = format!("{dir}/no_such_file_xyz{d}rs");

    // (1) STALE — the marker sits over one path, and that path is on disk. It silences nothing.
    let stale = scan_text("SYNTHETIC.md", &format!("- `{here}` is built. {PLANNED_MARKER}"));
    assert_eq!(
        stale.stale_planned.len(),
        1,
        "a `doc-path-planned` marker whose every path is already on disk waives nothing and must \
         be reported — this is the half of the contract that stops being observable at the exact \
         moment the work succeeds; got {:?}",
        stale.stale_planned
    );
    assert!(
        stale.planned_paths.is_empty(),
        "a path that EXISTS is not a planned deliverable; it must not also be counted as one: {:?}",
        stale.planned_paths
    );

    // (2) The live half, and the non-vacuity control. If the marker were not being parsed at all,
    // (1) would pass for the wrong reason and so would (3) — this is what proves the scan sees it.
    let doing_its_job = scan_text("SYNTHETIC.md", &format!("- `{gone}` is planned. {PLANNED_MARKER}"));
    assert!(
        doing_its_job.stale_planned.is_empty(),
        "a marker over a path that is genuinely missing is waiving something and must stay silent, \
         or documents get pushed to strip markers that are still load-bearing: {:?}",
        doing_its_job.stale_planned
    );
    assert_eq!(
        doing_its_job.planned_paths.len(),
        1,
        "the missing path must be waived into the planned ledger; 0 here means this fixture's \
         marker never parsed and (1) proves nothing"
    );
    assert!(
        doing_its_job.path_violations.is_empty(),
        "a waived missing path must not ALSO be a dead-path violation — that is what the marker \
         buys: {:?}",
        doing_its_job.path_violations
    );

    // (3) MIXED — the rule is `all`, not `any`. One missing path on the line still makes the
    // marker load-bearing, so the line is not stale even though another path on it exists.
    let mixed = scan_text(
        "SYNTHETIC.md",
        &format!("- `{here}` is built, `{gone}` is not. {PLANNED_MARKER}"),
    );
    assert!(
        mixed.stale_planned.is_empty(),
        "a marker is stale only when EVERY path on its line exists; one missing path keeps it \
         load-bearing, and reporting it would be a false accusation: {:?}",
        mixed.stale_planned
    );

    // (4) The no-marker control: the same existing path with no marker fires neither ledger.
    // Without this, (1) would pass identically if the scan reported every line it saw.
    let unmarked = scan_text("SYNTHETIC.md", &format!("- `{here}` is built."));
    assert!(
        unmarked.stale_planned.is_empty() && unmarked.planned_paths.is_empty(),
        "neither planned-path ledger may fire on a line that carries no marker: stale {:?}, \
         planned {:?}",
        unmarked.stale_planned,
        unmarked.planned_paths
    );
}

/// Every count of the tree **this file's own prose** states, re-taken from the tree on every run.
///
/// # Why this is not a marker, and why that is a property rather than an excuse
///
/// The measurement marker is the right instrument for this class and it **cannot be pointed at
/// this file**, for a reason that is structural and worth stating once so nobody spends another
/// round discovering it. [`measured_docs`] admits `.md` files under `docs/`; widening it to admit
/// this source would make [`measured_claims`] scan the very text that DEFINES the marker — the
/// constant holding its opening delimiter, and the fenced example of its grammar written three
/// lines below that constant. Both parse as malformed markers (one is never closed, the other ends
/// in a word where a number belongs), so the widened scanner reds on its own definition. Skipping
/// those lines by pattern would be a scanner with a hand-cut hole gating a corpus for hand-cut
/// holes, which is the shape this campaign exists to find. **A mechanism that reads a corpus cannot
/// have its own grammar inside that corpus.**
///
/// So the figures are re-derived directly instead, which for these seven is strictly stronger than
/// a marker: a marker carries a hand-written `= N` that a careless edit can update in lockstep with
/// the prose, and there is no `= N` here at all. The tree is the only source, and the prose is the
/// thing under test.
///
/// # What is checked, and the one thing that is not
///
/// Each row names a phrase that locates the sentence stating the figure, and a re-derivation. The
/// sentence must WRITE the re-derived number, by [`writes_number`] — the same visibility rule the
/// marker's proximity guard applies, and the same limit applies with it: a window containing
/// several integers passes if any of them matches, so this proves a correct figure is PRESENT, not
/// that a stale one is absent beside it.
///
/// The anchor phrases are matched only against lines that are doc comments. Without that filter the
/// lookup would resolve against this test's own table — the phrase is written twice, once as prose
/// and once as the key that finds it — and every row would certify itself. That is the
/// self-matching trap this file carries in three other places, and it is why the search is
/// restricted rather than made unique by wording.
///
/// # ⚠️ What forced this
///
/// Four figures in the block above were stale at once, and the landing that wrote them was the
/// landing that moved the tree underneath them. One said *"All four re-measure correctly"* over a
/// file that its own edit set had grown by 482 lines — and by 502 once the record pass that
/// followed had finished appending to it, which is the second half of the same point. One said
/// **3** where the answer is more than twice that, having been taken by recollection rather than by
/// a loop. **The figures that rot are the ones written while explaining the mechanism that would
/// have caught them**, and the block above is exactly that kind of prose.
///
/// ⚠️ **The quoted words above read *"all four re-measure correctly today"* until 2026-08-29, and
/// no document in this corpus ever contained that sentence** — the trailing word was supplied by
/// whoever repeated it, and then repeated twice more. It is a small thing and it is the same thing:
/// a claim written from recollection rather than from the source, inside a paragraph about claims
/// written from recollection. Quote verbatim or drop the quotation marks.
#[test]
fn figures_this_file_writes_about_the_tree_are_re_derived() {
    let root = repo_root();
    let own = std::fs::read_to_string(root.join("tests/internal_docs_anchors.rs"))
        .expect("invariant: this test's own source must be readable from the manifest directory");

    // `None` is "there was nothing to read", never "the answer is nought" — the distinction
    // `Measurement::take` had to grow after two markers were certified against absent files.
    //
    // ⚠️ **`wc -l` semantics, deliberately, and the first draft of this gate got it wrong.** The
    // bullet these rows check is titled with that command, and `str::lines()` is NOT the same
    // function: it counts a final unterminated line that `wc -l` does not. Measured while writing
    // this — the god-file has no trailing newline, so the two answers are 1917 and 1918, and a gate
    // built on `lines()` redded a figure that was CORRECT and would have had it "repaired" into a
    // number matching no command anyone runs. An instrument must measure the quantity the prose
    // names; measuring a neighbouring one and calling the difference rot is how a repair pass
    // introduces the defect it came to remove.
    let wc_lines = |rel: &str| -> Option<usize> {
        std::fs::read(root.join(rel))
            .ok()
            .map(|b| b.iter().filter(|&&c| c == b'\n').count())
    };
    let str_lines = |rel: &str| -> Option<usize> {
        std::fs::read_to_string(root.join(rel))
            .ok()
            .map(|t| t.lines().count())
    };

    // Lines of one document on which THIS file's name is followed by a colon and a digit. The
    // needle is assembled rather than spelled: written whole it would be a citation of this file
    // inside this file, and it would land against the caps of the two source censuses.
    let cites_this_file = |rel: &str| -> Option<usize> {
        let needle = format!("internal_docs_anchors{}rs:", '.');
        std::fs::read_to_string(root.join(rel)).ok().map(|t| {
            t.lines()
                .filter(|l| {
                    l.match_indices(needle.as_str()).any(|(i, _)| {
                        l.as_bytes()
                            .get(i + needle.len())
                            .is_some_and(u8::is_ascii_digit)
                    })
                })
                .count()
        })
    };

    // The two marker populations the limit paragraph counts. Per MARKER and not per LINE: two
    // markers on one line are two claims, and the sentence they annotate says "markers".
    let (claims, _) = measured_claims();
    let mut with_command = 0usize;
    let mut marker_only = 0usize;
    for c in &claims {
        let Ok(text) = std::fs::read_to_string(root.join(&c.doc)) else {
            continue;
        };
        let lines: Vec<&str> = text.lines().collect();
        let own_line = lines.get(c.line - 1).copied().unwrap_or("");
        // The SAME three-line window the proximity guard reads, so the limit paragraph's figure
        // and the guard it describes cannot be scoped differently.
        let window = [
            lines.get(c.line.wrapping_sub(2)).copied().unwrap_or(""),
            own_line,
            lines.get(c.line).copied().unwrap_or(""),
        ];
        if window.iter().any(|l| l.contains("grep")) {
            with_command += 1;
        }
        if strip_measure_markers(own_line).trim().is_empty() {
            marker_only += 1;
        }
    }

    let twin_en = "docs/OPEN-QUESTIONS.md";
    let twin_ru = "docs/ru/OPEN-QUESTIONS.md";
    let both_twins = match (cites_this_file(twin_en), cites_this_file(twin_ru)) {
        (Some(a), Some(b)) => Some(a + b),
        _ => None,
    };

    // (anchor phrase locating the sentence, what the figure is, the tree's answer)
    let figures: Vec<(&str, &str, Option<usize>)> = vec![
        (
            "Live in the corpus:",
            "lines of the by-id seam SOURCE",
            wc_lines("crates/boyko_ecs/src/ecs/core/ecs_master/seam_by_id.rs"),
        ),
        (
            "Live in the corpus:",
            "lines of the by-id seam TEST",
            wc_lines("crates/boyko_ecs/tests/seam_by_id.rs"),
        ),
        (
            "Inexpressible as a marker",
            "lines of the god-file the refactoring campaign split",
            wc_lines("crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs"),
        ),
        (
            "Inexpressible as a marker",
            "lines of docs/REFLECTION-PLAN-CORE.md",
            wc_lines("docs/REFLECTION-PLAN-CORE.md"),
        ),
        (
            "The census's OWN figures",
            "bound `.rs` citations the census binds",
            Some(rs_citations_in_rust_sources().0.len()),
        ),
        (
            "twins cite THIS file by line",
            "citations of this file in the EN twin",
            cites_this_file(twin_en),
        ),
        (
            "twins cite THIS file by line",
            "citations of this file in the RU twin",
            cites_this_file(twin_ru),
        ),
        (
            "twins cite THIS file by line",
            "citations of this file across both twins",
            both_twins,
        ),
        (
            "Making it a comparison was measured",
            "measurement markers in the corpus",
            Some(claims.len()),
        ),
        (
            "carry a machine-readable command",
            "markers with a command in the proximity window",
            Some(with_command),
        ),
        (
            "markers sit on lines whose",
            "markers on lines whose whole content is markers",
            Some(marker_only),
        ),
    ];

    let mut report = String::new();
    let mut checked = 0usize;
    for (anchor, what, got) in &figures {
        // Doc-comment lines only. The prose and this table write the same phrase, and an
        // unrestricted search resolves against the table — every row certifying itself.
        let own_lines: Vec<&str> = own.lines().collect();
        // The anchor locates a SENTENCE, and a sentence in a doc comment is wrapped across lines
        // at whatever column the last edit left. So the figure is looked for in the same
        // three-line window `writes_number`'s own guard reads, not on the anchor's exact line —
        // otherwise re-flowing a paragraph moves a number out of reach of the phrase that names it
        // and the gate reds on formatting. Measured: the god-file's figure and the phrase that
        // anchors it landed on adjacent lines the moment the paragraph above was rewritten.
        let found = own_lines
            .iter()
            .position(|l| l.trim_start().starts_with("///") && l.contains(anchor))
            .map(|i| {
                [
                    own_lines.get(i.wrapping_sub(1)).copied().unwrap_or(""),
                    own_lines[i],
                    own_lines.get(i + 1).copied().unwrap_or(""),
                ]
                .join("\n")
            });
        let (Some(line), Some(n)) = (found.as_deref(), *got) else {
            report.push_str(&match (found.is_some(), got.is_some()) {
                (false, _) => format!(
                    "  {what}: no doc-comment line contains the phrase that locates it. Either the \
                     sentence was rewritten — move the phrase here in the same edit — or a gated \
                     figure went back to being ungated prose.\n"
                ),
                (true, _) => format!(
                    "  {what}: there is NOTHING TO MEASURE — the file or population is absent. \
                     This is not the figure being confirmed; it is the measurement never \
                     happening.\n"
                ),
            });
            continue;
        };
        checked += 1;
        println!("  {what} -> {n}");
        if !writes_number(line, n) {
            report.push_str(&format!(
                "  {what}: the tree says {n} and the sentence stating it does not write that \
                 number.\n    {}\n",
                line.trim()
            ));
        }
    }

    // NON-VACUITY. A rewritten paragraph that dropped every phrase would leave the loop above
    // with nothing to disagree with, and an empty report is this gate's pass.
    assert_eq!(
        checked,
        figures.len(),
        "every row must have resolved to a sentence AND to a measurement; {} of {} did",
        checked,
        figures.len()
    );

    // The discrepancy the bullet above calls LOAD-BEARING, pinned rather than described. The
    // sentence claims one of these files distinguishes `wc -l` from `str::lines()`; that is a
    // property of the file on disk, and if someone appends a trailing newline the sentence quietly
    // stops being true while every figure beside it stays green. Asserted as an inequality plus
    // its exact size, because "they differ" and "they differ by the one final unterminated line"
    // are different claims and only the second is the one being made.
    let god_file = "crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs";
    assert_eq!(
        (wc_lines(god_file), str_lines(god_file)),
        (Some(1917), Some(1918)),
        "the `str::lines()`-versus-`wc -l` discrepancy this file's prose calls load-bearing is a \
         property of the god-file having NO final newline. If these are now equal the file gained \
         one and the sentence naming the discrepancy is stale; if both moved, the figure beside it \
         is"
    );

    // The discriminator for the mechanism itself, asserted directly rather than left to the
    // corpus: the doc-comment filter is the whole reason a row cannot certify itself, so it is
    // pinned where it is relied on. The phrase below is written ONLY here and in this assertion.
    assert!(
        own.lines()
            .any(|l| l.trim_start().starts_with("///") && l.contains("Live in the corpus:")),
        "the anchor lookup must find PROSE; if this fails the doc-comment filter is matching \
         nothing and every row above resolved against this table instead"
    );
    assert!(
        !"        (\"Live in the corpus:\",".trim_start().starts_with("///"),
        "a table row must NOT satisfy the doc-comment filter — that is what keeps a row from \
         answering its own lookup"
    );

    assert!(
        report.is_empty(),
        "this file states a count of the tree that the tree does not support.\n\
         These are the figures a marker cannot reach — file lengths, the census's own output, and \
         counts over the marker corpus itself — so they are re-derived here instead. Re-take the \
         figure and write the new one; do not adjust this test to agree with the prose.\n{report}"
    );
}
