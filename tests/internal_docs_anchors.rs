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
//! 1. **Paths** — every markdown link target `](...)` and every bare `crates/...` mention in
//!    prose or backticks must exist on disk.
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
//! * **Historical quotes** — a line that deliberately reproduces a former, now-wrong anchor
//!   ("this used to say ...") carries `<!-- doc-anchor-ignore -->` and is skipped whole. This is
//!   an explicit opt-out rather than a heuristic on words like "formerly": a heuristic would
//!   silently switch the gate off on ordinary lines that happen to use the word. No line in the
//!   three documents needs it today.
//!
//! # Scope
//!
//! The three navigation documents **and** `MESHLET-VIRTUAL-GEOMETRY-PLAN.md`. The rest of `docs/`
//! is audit and results files: dated records of what was believed at a point in time. Rewriting
//! their anchors to match today's source would falsify the record, so they stay out of scope.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The internal documents `CLAUDE.md` points agents at for navigation.
/// The three navigation documents, plus the virtual-geometry plan.
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
    let path = if base.starts_with("crates/") || base.starts_with("docs/") {
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

    // 2. Bare `crates/...` and `docs/...` mentions in prose or backticks.
    //
    // ⚠️ `docs/` was added with the register corpus. Those documents cite each other by
    // repo-relative path with a line number — `` `docs/OPEN-QUESTIONS.md:250` `` — and with only
    // `crates/` scanned, no mention was produced, so the `:250` bound to whatever SOURCE file was
    // cited last and was reported stale against it. Measured on OPEN-QUESTIONS.md:124, where
    // `docs/OPEN-QUESTIONS.md:250`, `:282` and `:198` were all charged to `light.rs`.
    for needle in [&b"crates/"[..], &b"docs/"[..]] {
    let mut i = 0;
    while i + needle.len() <= bytes.len() {
        if &bytes[i..i + needle.len()] == needle
            && !link_spans.iter().any(|&(s, e)| i >= s && i < e)
            // Only a path *start*: `subcrates/` and the tail of `../crates/` handled elsewhere.
            && (i == 0 || !is_path_byte(bytes[i - 1]))
        {
            let mut end = i;
            while end < bytes.len() && is_path_byte(bytes[end]) {
                end += 1;
            }
            // A GLOB is a pattern, not a claim that one file exists. `*` is not a path byte, so
            // the scan above stops in front of it and would otherwise report the truncated stem:
            // measured, `docs/PHASE-*-RESULTS.md` in FEATURE_MAP.md and SYSTEMS.md came back as
            // ``path does not exist: `docs/PHASE` `` — a failure manufactured out of correct prose.
            if end < bytes.len() && bytes[end] == b'*' {
                i = end;
                continue;
            }
            if let Some(slice) = text.get(i..end) {
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
            continue;
        }
        i += 1;
    }
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
                let closes =
                    !paren || matches!(bytes.get(after).copied(), Some(b')' | b',' | b'/'));
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
    const KEYWORDS: &[&str] = &[
        "fn ",
        "struct ",
        "enum ",
        "const ",
        "static ",
        "impl ",
        "trait ",
        "type ",
        "pub ",
        "macro_rules!",
        "union ",
        "mod ",
    ];
    KEYWORDS.iter().any(|kw| {
        t.match_indices(kw).any(|(idx, _)| {
            // Word boundary: the keyword must not be the tail of a longer identifier.
            idx == 0
                || !t.as_bytes()[idx - 1].is_ascii_alphanumeric() && t.as_bytes()[idx - 1] != b'_'
        })
    })
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
    /// of a document that shrank — is still caught, and was: `gaia/CAMPAIGN.md:1262` in a file of
    /// 339 lines. Counted separately so a corpus that quietly became all-prose cannot look
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
    evidence: bool,
    file_lines: &mut BTreeMap<PathBuf, Option<Vec<String>>>,
    out: &mut Vec<String>,
    over_waived: &mut Vec<String>,
    relaxed: &mut usize,
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
    let doc_dir = doc_dir_of(doc);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("gated internal doc {} is unreadable: {e}", path.display()));

    let mut mentions = 0usize;
    let mut anchors = 0usize;
    let mut classes = [0usize; 5];
    let mut path_violations = Vec::new();
    let mut anchor_violations = Vec::new();
    let mut over_waived = Vec::new();
    let mut evidence_relaxed = 0usize;
    let evidence = EVIDENCE_DOCS.contains(&doc);

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

    // The register's own retraction convention: text inside `~~ … ~~` is STRUCK, and a struck
    // citation is the record of a claim that was withdrawn, not a live claim. It keeps its old
    // line number on purpose (2026-09-10: fourteen ruled ballots and one refuted mechanism were
    // struck in place with dated pointers rather than deleted). Checking a struck anchor for
    // identity would red on every honest retraction, so struck spans are blanked before the
    // scan — blanked, not removed, so every line number below still means what it means.
    let text = blank_struck(&text);
    let text = text.as_str();

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
                    evidence,
                    &mut file_lines,
                    &mut anchor_violations,
                    &mut over_waived,
                    &mut evidence_relaxed,
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

        let (line_mentions, line_anchors) = scan_line(line, &doc_dir);
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
        // `bound` tracks that binding for THIS line, so a fragment written directly in front of an
        // anchor can override it. ⚠️ Without that override, an anchor whose own file name is not
        // resolvable as a path binds to whatever was cited last and is judged against a file it
        // never named — the largest false class in the first widened run. Measured on
        // OPEN-QUESTIONS.md:497-498, where `solver/colored.rs:2667`, `soft/colored.rs:1031`,
        // `resources.rs:1688` and `:1760` were all charged to a `worker.rs` link from fourteen
        // lines above and reported "past end of file (1397 lines)" — four findings, none of them
        // about any of those four files. A fragment that DOES resolve rebinds (the fence rule,
        // applied outside a fence); one that does not CLEARS the binding, so the anchor and any
        // bare `:N` continuation after it are skipped rather than misattributed.
        let mut mi = 0usize;
        let mut bound: Option<(String, PathBuf)> = current
            .as_ref()
            .filter(|(_, _, exists)| *exists)
            .map(|(raw, p, _)| (raw.clone(), p.clone()));
        for (ai, anchor) in line_anchors.iter().enumerate() {
            while mi < line_mentions.len() && line_mentions[mi].col < anchor.col {
                let m = &line_mentions[mi];
                mentions += 1;
                if !m.resolved.exists() {
                    path_violations.push(format!(
                        "  {doc}:{lineno}  path does not exist: `{}`",
                        m.raw
                    ));
                } else if m.resolved.starts_with(&crates_root) {
                    fence_base = Some(m.resolved.clone());
                }
                if is_file_shaped(&m.raw) {
                    current = Some((m.raw.clone(), m.resolved.clone(), m.resolved.is_file()));
                    bound = m
                        .resolved
                        .is_file()
                        .then(|| (m.raw.clone(), m.resolved.clone()));
                }
                mi += 1;
            }

            if let Some(frag) = fragment_before(line, anchor.col) {
                bound = resolve_fragment(frag, current.as_ref().map(|c| &c.1), fence_base.as_ref())
                    .map(|p| (frag.to_string(), p));
            }

            let Some((raw, target)) = bound.as_ref() else {
                // No anchor target since the last heading, or one this anchor named and the
                // scanner could not resolve: nothing it may honestly be checked against.
                continue;
            };
            let expect = paired.then(|| idents[ai].as_str());
            if let Some(class) = check_anchor(
                doc,
                lineno,
                raw,
                target,
                anchor,
                expect,
                evidence,
                &mut file_lines,
                &mut anchor_violations,
                &mut over_waived,
                &mut evidence_relaxed,
            ) {
                anchors += 1;
                classes[class as usize] += 1;
            }
        }

        // Mentions to the right of the last anchor (or all of them, on a line with none).
        for m in &line_mentions[mi..] {
            mentions += 1;
            if !m.resolved.exists() {
                path_violations.push(format!(
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
        evidence_relaxed,
        over_waived,
    }
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
