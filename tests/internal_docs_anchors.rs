//! Gate for the internal navigation docs: every path they cite must exist, and every line
//! anchor — `file.rs:N` or the bare `(N)` of a member table — must land on the definition it
//! claims.
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
//! 5. **Non-emptiness, per document** — a mis-typed pattern or a renamed document must fail
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
//!   loosened for everyone. 25 anchors carry it today, on 15 lines — struct fields
//!   (`Archetype::enable_store`, `ObserverLists::by_kind_component`,
//!   `ArchetypeMaster::observer_registry`), enforcement sites inside a derive body, the four call
//!   sites of the enable-store 0%-gate, and the `dispatch.rs` OBS-FIRE-LOOP module-doc invariant.
//!   Enumerate them from `docs/` with
//!   `grep -oE '[:(][0-9]+(-[0-9]+)?~' FEATURE_MAP.md SYSTEMS.md ARCHITECTURE.md | wc -l`. Both
//!   loosenings in that pattern are load-bearing and each was measured: dropping the `[:(]`
//!   alternative to a bare `:` reports 24, because one waiver is written in the parenthesised
//!   form (`(65~`, FEATURE_MAP.md:756); dropping `(-[0-9]+)?` also reports 24, because one is
//!   written on a range (`dispatch.rs:19-33~`, SYSTEMS.md:435). Use `-o`, not `-n`: several of
//!   these lines carry more than one waiver, so counting lines reports 15, not 25.
//!   ⚠️ **All four numbers here were 26/16/25/25 and went stale in the commit that un-waived one
//!   anchor in SYSTEMS.md** — a repair that changed a measured INPUT rather than restating a fact,
//!   so grepping for the sentence would not have found them. They are re-derived, not adjusted.
//!   That pattern reads only the three navigation documents, and it is deliberately NOT the
//!   whole-corpus census: the meshlet plan writes its waivers before the range (`:N~-M`), a
//!   spelling with zero occurrences in these three. To enumerate every waiver in `GATED_DOCS`,
//!   accept the tilde on either side — `grep -oE '[:(][0-9]+~(-[0-9]+)?|[:(][0-9]+(-[0-9]+)?~'` —
//!   and run it over all nine.
//! * **Historical quotes** — a line that deliberately reproduces a former, now-wrong anchor
//!   ("this used to say ...") carries `<!-- doc-anchor-ignore -->` and is skipped whole. This is
//!   an explicit opt-out rather than a heuristic on words like "formerly": a heuristic would
//!   silently switch the gate off on ordinary lines that happen to use the word.
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
    "REFLECTION-ANALYSIS.md",
    "REFLECTION-PLAN-BOUNDARY.md",
    "REFLECTION-PLAN-CORE.md",
    "REFLECTION-PLAN-ECS.md",
    "REFLECTION-PLAN-GATES.md",
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

/// Repo-root-relative prefixes a **bare** (unlinked) path mention may start with.
///
/// This was `crates/` alone, and the omission was measured when the reflection documents entered
/// `GATED_DOCS`: they cite `.github/workflows/ci.yml:62` and `docs/FEATURE_MAP.md:112` in prose,
/// neither of which the scan could see. An unseen mention is not a skipped check — it is a
/// **misbinding**, because the anchor after it falls through to the last `crates/...` path instead,
/// and the run then reports that file's line count. `REFLECTION-PLAN-GATES.md:1684` cited seven
/// `ci.yml` legs and every one was checked against `crates/profile_fixture/Cargo.toml` (18 lines).
///
/// ⚠️ **That `:1684` is prose, and nothing checks it.** It was written `:1641`, was already off by
/// two when written, and drifted to `:1684` when a discharge banner went in above it — found by
/// reading, not by a run. The reason is structural and worth naming rather than repairing twice:
/// **this census scans `.md` files for citations into `.rs`, never the reverse.** A `.rs` file
/// citing a `.md` line — this comment, and any other — is outside every census in the tree, so its
/// line numbers rot silently. Recorded as a known gap, not scheduled: the reverse direction is a
/// second scanner with its own false-binding surface, and the citations it would cover are
/// explanatory rather than load-bearing.
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
/// Markdown link targets are relative to the document, i.e. to `docs/`; bare `crates/...`
/// mentions in prose are written from the repository root.
fn resolve(raw: &str) -> Option<PathBuf> {
    if raw.starts_with("http://") || raw.starts_with("https://") || raw.starts_with("mailto:") {
        return None;
    }
    // Strip a `#fragment` — the anchor within a document is not part of the file path.
    let base = raw.split('#').next().unwrap_or("");
    if base.is_empty() {
        return None;
    }
    let mut path = if let Some(rest) = base.strip_prefix("../") {
        repo_root().join(rest)
    } else if ROOT_PREFIXES.iter().any(|p| base.starts_with(p)) {
        repo_root().join(base)
    } else {
        docs_dir().join(base)
    };
    // `..` segments beyond the first are rare but must not defeat the existence check.
    if base.contains("/../") {
        path = normalize(&path);
    }
    Some(path)
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
fn scan_line(text: &str) -> (Vec<Mention>, Vec<Anchor>) {
    let bytes = text.as_bytes();
    let mut mentions: Vec<Mention> = Vec::new();
    // Byte spans already consumed by a markdown link target, so the bare-mention scan below does
    // not report the same path a second time.
    let mut link_spans: Vec<(usize, usize)> = Vec::new();

    // 1. Markdown link targets: `](target)` or `](target "title")`.
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b']' && bytes[i + 1] == b'(' {
            let start = i + 2;
            if let Some(rel_end) = bytes[start..].iter().position(|&b| b == b')') {
                let end = start + rel_end;
                if let Some(inner) = text.get(start..end) {
                    // A link title after the target is not part of the path.
                    let target = inner.split_whitespace().next().unwrap_or("");
                    link_spans.push((start, end));
                    if !target.is_empty()
                        && !target.starts_with('#')
                        && let Some(resolved) = resolve(target)
                    {
                        mentions.push(Mention {
                            col: start,
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
                        && let Some(resolved) = resolve(raw)
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
            b'(' => !(i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_')),
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

/// The `file.rs` fragment written immediately left of a fenced anchor's colon, if any.
///
/// `// mod.rs:46` yields `mod.rs`; `(enable_store.rs:259)` yields `enable_store.rs`; a bare
/// `// :59` yields `None` and the anchor falls back to the fence's current target.
fn fragment_before(text: &str, col: usize) -> Option<&str> {
    let bytes = text.as_bytes();
    let mut start = col;
    while start > 0 && is_path_byte(bytes[start - 1]) {
        start -= 1;
    }
    let frag = text.get(start..col)?;
    if frag.is_empty() || Path::new(frag).extension().is_none() {
        return None;
    }
    Some(frag)
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
            return None;
        }
        dir = dir.parent()?.to_path_buf();
        if !dir.starts_with(&root) {
            return None;
        }
    }
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
/// The four are exhaustive and disjoint, so they sum to the anchor count.
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
}

/// Everything one document yields, after fences and ignored lines are removed.
struct DocScan {
    mentions: usize,
    anchors: usize,
    /// Anchor count split by [`AnchorClass`], in declaration order.
    classes: [usize; 4],
    path_violations: Vec<String>,
    anchor_violations: Vec<String>,
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
}

impl DocScan {
    fn count(&self, class: AnchorClass) -> usize {
        self.classes[class as usize]
    }
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
    file_lines: &mut BTreeMap<PathBuf, Option<Vec<String>>>,
    out: &mut Vec<String>,
    over_waived: &mut Vec<String>,
) -> Option<AnchorClass> {
    let entry = file_lines.entry(target.to_path_buf()).or_insert_with(|| {
        std::fs::read_to_string(target)
            .ok()
            .map(|s| s.lines().map(str::to_string).collect())
    });
    let src = entry.as_ref()?;

    // Classified before the bounds test, so the decomposition describes the reach of the gate
    // rather than which anchors happen to be failing today.
    let class = if anchor.shape_waived {
        AnchorClass::Waived
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
    if !looks_like_definition(src_line, ext) {
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

fn scan_doc(doc: &str) -> DocScan {
    let path = docs_dir().join(doc);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("gated internal doc {} is unreadable: {e}", path.display()));

    let mut mentions = 0usize;
    let mut anchors = 0usize;
    let mut classes = [0usize; 4];
    let mut path_violations = Vec::new();
    let mut anchor_violations = Vec::new();
    let mut over_waived = Vec::new();
    let mut unbindable = Vec::new();
    let mut planned_paths = Vec::new();
    let mut stale_planned = Vec::new();

    // Cache of file contents, so a document citing one file 15 times reads it once.
    let mut file_lines: BTreeMap<PathBuf, Option<Vec<String>>> = BTreeMap::new();
    // The sticky anchor target: the last file-shaped mention seen, reset at each heading.
    let mut current: Option<(String, PathBuf, bool)> = None;
    // The source-tree path a fenced margin-note fragment resolves against. Unlike `current` it
    // accepts directories (a section's `**Files:** [core/system/]` is exactly what `system/
    // system.rs` inside the fence below it is written relative to) and it survives sub-headings,
    // because `### 9.2` inherits the file list of `## 9`.
    let mut fence_base: Option<PathBuf> = None;
    let crates_root = repo_root().join("crates");
    let mut in_fence = false;
    // Inside a fence: the file its margin notes currently point into.
    let mut fence_target: Option<(String, PathBuf)> = None;

    for (idx, line) in text.lines().enumerate() {
        let lineno = idx + 1;
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            if in_fence {
                // A fence opens on its section's file, so a bare `:N` before the first margin
                // note still has a target.
                fence_target = current
                    .as_ref()
                    .filter(|(_, _, exists)| *exists)
                    .map(|(raw, p, _)| (raw.clone(), p.clone()));
            }
            continue;
        }
        if line.contains(IGNORE_MARKER) {
            continue;
        }
        // A deliverable the plan has not built yet is not a dead path. See `PLANNED_MARKER`.
        let planned = line.contains(PLANNED_MARKER);

        if in_fence {
            // Paths are not checked inside a fence — an ASCII tree or an example command is not
            // a claim that a file exists — but the margin notes ARE line claims, and they rot
            // exactly like the prose ones.
            let (_, line_anchors) = scan_line(line);
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
                if let Some(frag) = fragment_before(line, anchor.col)
                    && let Some(resolved) =
                        resolve_fragment(frag, current.as_ref().map(|c| &c.1), fence_base.as_ref())
                {
                    fence_target = Some((frag.to_string(), resolved));
                }
                let Some((raw, target)) = fence_target.as_ref() else {
                    continue;
                };
                if let Some(class) = check_anchor(
                    doc,
                    lineno,
                    raw,
                    target,
                    anchor,
                    decl.as_deref(),
                    &mut file_lines,
                    &mut anchor_violations,
                    &mut over_waived,
                ) {
                    anchors += 1;
                    classes[class as usize] += 1;
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

        let (line_mentions, line_anchors) = scan_line(line);
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
                    current = Some((m.raw.clone(), m.resolved.clone(), m.resolved.is_file()));
                }
                mi += 1;
            }

            // A fragment written immediately left of the anchor is the document's own statement of
            // which file it means, and it OVERRIDES the sticky binding. Where the two agree the
            // sticky one is kept (it is the fuller path); where the fragment names something else,
            // it is resolved on its own, and if it cannot be resolved uniquely the anchor is
            // SKIPPED. Falling back to the sticky binding here is precisely what checked
            // `component_registry/mod.rs:918` against `crates/boyko_ecs/Cargo.toml`.
            if let Some(frag) = fragment_before(line, anchor.col) {
                // The suffix must land on a path separator. Without that boundary `tags.rs` would
                // "agree" with a sticky `component_tags.rs`, and the anchor would be checked
                // against a file the document did not name — the very failure this override exists
                // to remove.
                let sticky_agrees = current
                    .as_ref()
                    .is_some_and(|(_, p, _)| path_ends_with_fragment(p, frag));
                if !sticky_agrees {
                    match resolve_unique_fragment(frag) {
                        Some(p) => current = Some((frag.to_string(), p, true)),
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

            let Some((raw, target, exists)) = current.as_ref() else {
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
                &mut file_lines,
                &mut anchor_violations,
                &mut over_waived,
            ) {
                anchors += 1;
                classes[class as usize] += 1;
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
                current = Some((m.raw.clone(), m.resolved.clone(), m.resolved.is_file()));
            }
        }
    }

    DocScan {
        mentions,
        anchors,
        classes,
        path_violations,
        anchor_violations,
        planned_paths,
        stale_planned,
        unbindable,
        over_waived,
    }
}

fn scan_all() -> BTreeMap<&'static str, DocScan> {
    GATED_DOCS.iter().map(|d| (*d, scan_doc(d))).collect()
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
        "internal navigation docs cite paths that do not exist.\n\
         These documents are the mandated first point of contact (CLAUDE.md); a dead path sends \
         the reader nowhere.\n{report}"
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
        assert_eq!(
            waived + unpaired + undeclared + identity,
            scan.anchors,
            "docs/{doc}: the class histogram does not account for every counted anchor — the \
             decomposition is the only thing that makes the coverage claim checkable, so a \
             mismatch here means the claim is unbacked."
        );
        println!(
            "docs/{doc}: {} anchors = {identity} identity-asserted + {unpaired} shape-only \
             (unpaired) + {undeclared} shape-only (symbol not declared in the cited file) + \
             {waived} waived (`~`: neither shape nor identity, only the in-file bounds check)",
            scan.anchors
        );
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
        "internal navigation docs cite line numbers that no longer hold a definition.\n\
         Re-derive each anchor from the current source. If an anchor deliberately points at a \
         non-definition line, mark it `:N~` / `(N~)` rather than loosening the check.\n{report}"
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
        let (_, anchors) = scan_line(text);
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
        let n = text
            .lines()
            .map(|l| {
                scan_line(l)
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
        for a in scan_line(line).1.iter().filter(|a| a.range_end.is_some()) {
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
    let (mentions, anchors) = scan_line("`--exclude boyko_demo` at `.github/workflows/ci.yml:62`");
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
        scan_line("its record is `docs/PHASE-*-RESULTS.md`.").0.len(),
        0,
        "a glob names a FAMILY. Read as a path it truncates at the `*` to `docs/PHASE-`, then \
         `docs/PHASE` — a file that was never claimed to exist, reported dead in two documents."
    );
    assert_eq!(
        scan_line("see `docs/FEATURE_MAP.md` for the map").0.len(),
        1,
        "rejecting globs must not reject literal paths under the same prefix"
    );

    // --- A ratio is not an anchor; a `(a) / (b)` list still is. ---
    assert_eq!(
        scan_line("the two targets sum to exactly 2.000 (3840/1920 = 2.000)").1.len(),
        0,
        "`(3840/1920` was parsed as an anchor on line 3840. The `/` alternative is for a member \
         list separator, never for a slash sitting directly between two digits."
    );
    let list = scan_line("`spawn_one` (582) / `spawn_batch` (611)").1;
    assert_eq!(
        list.len(),
        2,
        "rejecting ratios must not break the `(a) / (b)` member list the `/` was added for"
    );
    assert_eq!((list[0].line_no, list[1].line_no), (582, 611));

    // --- A prose enumerator is not a citation; a trailing member ref still is. ---
    assert_eq!(
        scan_line("**Landed notes (2026-08-21, recorded at execution).** (1) Gate 5 requires the")
            .1
            .len(),
        0,
        "`(1)` opening an enumerated clause is not an anchor on line 1. Reading it as one produced \
         four `stale` reds in REFLECTION-PLAN-GATES.md, and the first repair pass answered them by \
         writing `(1~)` `(2~)` `(3~)` INTO the prose — a scanner false positive laundered into the \
         document as a deliberate citation."
    );
    assert_eq!(
        scan_line("the blindness as (1) and the reason (2) is carried").1.len(),
        0,
        "a numbered reference back to an earlier item is prose, not a citation"
    );
    assert_eq!(
        scan_line("G0's census (3 tests), G1 (7), G2 (2), G3 (2, the").1.len(),
        0,
        "counts in parentheses are quantities; `(2,` even satisfies the comma continuation"
    );
    let member = scan_line("| everything | `clear()` (1026) |").1;
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
        ("REFLECTION-PLAN-ECS.md", 0), // 1 → 0 on 2026-08-26: EG1 built `ecs_alloc.rs`, the last marker this document carried. (2 → 1 the same day: EG0 built `seam_census.rs`.) Kept on ONE line: a bare `:1906-1941` fragment below cites this file and shifts silently.
        // 4 → 3 on 2026-08-26: CORE C9 built G5's `reflect_compile_fail.rs`, so its
    // `doc-path-planned` marker came off in the same edit as this decrement.
    ("REFLECTION-PLAN-GATES.md", 3),
        ("SYSTEMS.md", 0),
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
        println!("docs/{doc}: {n} anchor(s) skipped for an unresolvable fragment (cap {cap})");
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
