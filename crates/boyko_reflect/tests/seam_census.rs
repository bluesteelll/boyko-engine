//! **ECS EG0 — the seam census: compile the reachability claim before writing glue.**
//!
//! `docs/REFLECTION-PLAN-ECS.md` §4 of the analysis asserts that `add_default` / `remove`
//! *"route through the existing structural insert/remove"*. That plan's §1 facts **F9** and
//! **F11** say they cannot. The difference between those two sentences is three rungs of
//! work, and the cheapest way to know which one is true is to make the compiler say it —
//! before EG6 discovers it halfway through.
//!
//! The facts are cited **by anchor, never by copy**: §1 of
//! `docs/REFLECTION-PLAN-ECS.md` owns the F-rows and their `file:line`s, and this header
//! names them (F3, F4, F7, F9, F11, F16, F18, F20, F27) rather than reproducing them. A
//! table copied into a `.rs` header is covered by **no** census — `internal_docs_anchors`'s
//! `GATED_DOCS` is `.md`-only — so a copy would become a second, un-gated carrier of the
//! facts the anchors exist to keep single (EG0 gate 4 / D19).
//!
//! # The two halves, and why each is shaped the way it is
//!
//! **The positive half** ([`the_glues_route_is_reachable_from_this_package`]) *calls* the
//! accessors the glue will call, in the order the glue will call them, from a package that
//! is not `boyko_ecs`. Compiling is the claim; running is the bonus. The subject is a
//! **dynamic tag** (`register_tag` → `add_tag`), because EG0 *"constructs no reflect
//! component"* and this package deliberately carries no `boyko-macros` edge — a dynamic tag
//! is the one `Table` citizen constructible here, and F20 makes it a first-class row of §3's
//! matrix rather than a stand-in.
//!
//! ⚠️ **`is_enabled_id` / `enable_id` / `disable_id` are pinned by SIGNATURE, not by call
//! (D16).** All three compile and run today *given an `EnableTagId`* — so a row that mints
//! one with `register_enable_tag` and calls them proves nothing about the property this rung
//! exists to compile. What is unreachable is the **`ComponentId` → `EnableTagId` direction**
//! (F16: no reverse constructor), which is exactly what **S4′** adds and which lives on the
//! `compile_fail` side below until EG2. Coercing each to a `fn` pointer pins the item *and*
//! its signature without minting anything, and `register_enable_tag` is therefore **not** on
//! this census (D16: EG3 gate 7 asserts the glue never calls it, and a census certifying the
//! reachability of a call a gate forbids is a row that argues with a gate).
//!
//! **The negative half** ([`the_not_yet_reachable_seam_items_still_do_not_compile`]) is a
//! `trybuild` corpus: one fixture per item on the not-yet-reachable list. The list holds
//! **two kinds** and the distinction is load-bearing — the four things §4 must *add* (S1,
//! S2, S3, S4′), whose fixtures **flip to `pass` at EG2**, and the one thing the plan
//! *refuses* to add (`TagId::from_component_id`), whose fixture must stay red forever. A
//! single undifferentiated "negative list" is what let one item sit on it while a sibling
//! plan declared the same item mandatory.
//!
//! # Four ways this file could have been a gate that cannot fail — and what stops each
//!
//! 1. **An empty glob is a VACUOUS PASS**, MEASURED in this very package: `trybuild` prints
//!    *"There are no trybuild tests enabled yet"*, the harness reports `running 1 test … ok`,
//!    and the process exits **0**. `running N` is blind to it. So the glob states a floor —
//!    and the floor is **read from the plan**, not written here as a literal.
//! 2. **A floor that counts a DIFFERENT directory from the one the glob compiles guards
//!    nothing**, and the first cut of this file had exactly that shape: [`CORPUS`] was what
//!    the floor counted, while the harness spelled the directory a *second* time as a literal
//!    inside `t.compile_fail("tests/seam_compile_fail/*.rs")` — so the constant had one
//!    reader and it was not the glob. MEASURED: mutating **only** the glob by one character,
//!    directory and fixtures untouched, left the target at exit **0**, `3 passed`, with
//!    `trybuild` printing the very sentence item 1 quotes as the hazard. Zero fixtures were
//!    compiled, the `.stderr` corpus was never shown to a compiler, and the correspondence
//!    below stayed green because it reads files from disk rather than from `trybuild`. The
//!    glob is now built **from** [`CORPUS`]: one spelling, one reader. The same decoupling was
//!    inherited from `crates/reflect_fixture/tests/reflect_compile_fail.rs` (repaired in the
//!    same edit), and `tests/trybuild_corpus_compiler_witness.rs` now gates the whole class —
//!    every `trybuild` glob in the tree must resolve to at least one fixture.
//! 3. **Two constants declared in the same file, compared to each other**, cannot fail. So
//!    the correspondence between the plan's seam list and this corpus is asserted against
//!    `docs/REFLECTION-PLAN-ECS.md` **parsed at run time** (in-tree precedent:
//!    `crates/boyko_app/tests/vg_r0d_census.rs:56`).
//! 4. **A parser that matches nothing** asserts over an empty set and passes. So
//!    [`seam_items`] states its own non-vacuity floor — §4's table has four `S`-rows and EG0
//!    refuses exactly one item — and reds if the document stops yielding them.
//!
//! # The binding between a plan row and its fixture, and the two ways a substring failed
//!
//! The binding is the fixture's **blessed `.stderr`**, never its filename and never a comment.
//! What it looks for is the one sentence rustc writes *only* about an item that is **absent**
//! — ``error[E0599]: no … named `<fn>` found for … `<Type>` … in the current scope`` —
//! assembled by [`says_the_compiler_looked_and_did_not_find`] from the `Type::fn` spelling the
//! plan itself carries.
//!
//! ⚠️ **This was `err.contains(&item.path)` first, and a substring of the path is NOT that
//! property. Both failure directions were MEASURED at the EG0 verification, 2026-08-26.**
//!
//! * **A comment satisfies it.** rustc renders the *entire source line* of an error span,
//!   trailing comments included, so fixture-authored text lands inside the byte-checked
//!   diagnostic. Replacing S3's call with
//!   `let _ = EcsMaster::zqq_probe(&mut ecs, entity, id); // EcsMaster::mark_component_changed`
//!   and re-blessing gave exit **0**, `running 3 tests`, `3 passed`, five fixtures `ok` — while
//!   the compiler's actual finding was ``no associated function or constant named `zqq_probe` ``
//!   and **nothing anywhere had looked for `mark_component_changed`**. The plan row was bound
//!   to its fixture by a comment: the exact direction the assertion's own message calls
//!   dangerous.
//! * **Reachability satisfies it — the sharper direction.** The path can be in the diagnostic
//!   *because the item EXISTS*. With a three-argument `add_component_by_id` stub in the kernel,
//!   S1's fixture stopped being `E0599` and became `error[E0061]: this function takes 3
//!   arguments but 4 arguments were supplied`, whose machine-applicable suggestion prints
//!   `let _ = EcsMaster::add_component_by_id(&mut ecs, entity, id);` — path-qualified, and
//!   produced *precisely by* reachability. `contains` matched it **three** times in that one
//!   file. So the old binding was satisfiable by the exact opposite of what this census
//!   certifies, and that `.stderr` is one an EG2 implementer would bless on the way to landing
//!   S1.
//!
//! `E0599` is the compiler's code for *resolution looked and found nothing*, and its wording
//! names both halves of the pair the plan spells. Neither mutation above can produce that line
//! for the item the plan names, which is the whole of why it is the thing matched.
//!
//! ⚠️ **A blessed `.stderr` here embeds `boyko_ecs`'s own source text** (D18): rustc's
//! "similar name" suggestions are computed over `EcsMaster`'s entire inherent method set, so
//! a rename or a reformat in the kernel re-blesses these fixtures for a reason unrelated to
//! the seam. `tests/trybuild_corpus_compiler_witness.rs` freezes the compiler, not the method
//! table. This fails **loud**, so it is a maintenance cost, not a false green — recorded so
//! the re-bless is expected rather than investigated. Never re-bless without first confirming
//! `rustc --version`: a shadowing standalone `rustc 1.95.0` blesses a corpus the mandated
//! 1.97.1 then rejects.
//!
//! # Invocation
//!
//! ```text
//! cargo test -p boyko-reflect --test seam_census
//! ```
//!
//! **Plain — there is no feature leg to name.** This package declares no `[features]` table,
//! now or ever (GATES D4), and the census constructs no reflect component, so it is the one
//! glue test that belongs in `boyko_reflect`'s own tests.
//!
//! `#![cfg(not(miri))]` for **two** reasons, both measured (D19). The trybuild harness shells
//! out to `cargo`, which Miri cannot execute — `tests/c6_compile_fail.rs` already carries the
//! guard in this package for exactly that. And the run-time document parse is host file I/O,
//! which Miri refuses under isolation (`CreateFileW not available when isolation is enabled`,
//! measured at GATES G4's fifth RED and recorded at
//! `crates/boyko_reflect/tests/c2_registry_source_census.rs:12~`). CI runs
//! `cargo +nightly miri test --all-targets … -p boyko-reflect` (`.github/workflows/ci.yml:306`),
//! so `--all-targets` picks this target up; the CI comment block naming the hazard names
//! `reflect_fixture`'s harnesses, not this package's.
#![cfg(not(miri))]

use std::path::PathBuf;

use boyko_ecs::ecs::core::component::component_registry::{
    EnableTagId, ResidencyKind, StorageKind, get_layout, residency_class, storage_kind,
};
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_ecs::prelude::{EcsMaster, Entity};

// ═══════════════════════════════════════════════════════════════════════════════════════
// The positive half — the glue's route, called from a package that is not `boyko_ecs`
// ═══════════════════════════════════════════════════════════════════════════════════════

/// **EG0 gate 1.** Every accessor on the glue's own route resolves and composes from
/// `boyko_reflect`, against `boyko_ecs`'s **public** surface only.
///
/// The route is D1/D2/D14's, walked in order: entity → archetype id → archetype →
/// `component_ids()` (source 1, F7 — never `EntityInland::archetype_ptr()`, which F8 makes
/// inexpressible from an external crate anyway); `dense_registry().dense_ids()` ×
/// `dense_contains` (source 2, F4/F3); then the per-id classification and byte access.
///
/// `dense_slot_of` and `dense_get_raw` are censused on the corrected framing (D16): **no**
/// rung consumes them — dense reads go through `get_component_raw`, whose dense arm calls
/// `dense_get_raw` *inside* `boyko_ecs` — but F3 asserts all three are `pub` on `EcsMaster`,
/// and this is where that fact is compiled. They are rows of the route's *substrate*, not of
/// its callers, which is what the struck *"the accessors it intends to use"* framing could
/// not express.
#[test]
fn the_glues_route_is_reachable_from_this_package() {
    let mut ecs = EcsMaster::new();

    // The subject. `register_tag` is idempotent by name (F18) and `tag_by_name` resolves
    // without minting (F27's second half) — both are fixture setup here, and both are on the
    // glue's route: EG3's dynamic-tag write path is `tag_by_name(display_name(id))` →
    // `add_tag` / `remove_tag`, with no new API at all.
    let tag = ecs.register_tag("eg0_seam_census_subject");
    assert_eq!(
        ecs.tag_by_name("eg0_seam_census_subject"),
        Some(tag),
        "F18/F27: an interned dynamic-tag name resolves to its TagId without minting -- the \
         whole dynamic-tag presence WRITE path rests on this, and it is why the seam in §4 is \
         four items and not five"
    );
    let id: ComponentId = tag.component_id();

    let entity: Entity = ecs.spawn_empty();
    ecs.add_tag(entity, tag);

    // ── source 1 (D2): the safe `&self` accessor chain, F7 ──────────────────────────────
    let archetype_id = ecs
        .entity_archetype_id(entity)
        .expect("invariant: a live entity that has just been given a tag has an archetype");
    let archetype = ecs
        .archetype_master()
        .get_archetype(archetype_id)
        .expect("invariant: an ArchetypeId handed back by entity_archetype_id resolves");
    let signature: &[ComponentId] = archetype.component_ids();
    assert!(
        signature.contains(&id),
        "F5/F7: a dynamic tag is Table-kind, so it IS in the archetype signature -- source 1 \
         enumerating it is the premise EG1 builds `components_of_into` on"
    );

    // ── source 2 (D14): the dense registry, F4 + F3 ─────────────────────────────────────
    // The claim under test is REACHABILITY of the pair, not its cardinality: this package
    // carries no `boyko-macros` edge and so registers no dense component. EG1 gate 1 is where
    // a dense id is actually enumerated, on the fixture's shapes.
    let dense_ids: &[ComponentId] = ecs.dense_registry().dense_ids();
    println!("EG0 seam census: dense_ids().len() = {} in this binary", dense_ids.len());

    // ⚠️ MEASURED, and it is why there is no membership COUNT here. `dense_ids()` is EMPTY in
    // this binary, so `dense_ids.iter().filter(|&&d| ecs.dense_contains(entity, d)).count()`
    // iterates ZERO elements: the closure never runs, `dense_contains` is never invoked, and
    // `== 0` then holds for every possible implementation of it. That is `x || !x` spelled as
    // an iterator -- and the first cut of this file shipped exactly that, under a message
    // claiming it was the shape being kept OUT of the ladder. Three falsifiable statements
    // replace it, and none of them is the count.
    //
    // 1. The cardinality itself, which IS checkable and IS a claim about this binary.
    assert!(
        dense_ids.is_empty(),
        "dense_ids() is no longer empty in this binary ({} id(s)). Nothing here registers a dense \
         component, so this is either a new edge in `boyko_reflect`'s test dependencies or a \
         change in when `boyko_ecs` populates the registry. Either way the membership question \
         stopped being vacuous and now wants a real assertion -- EG1 gate 1 is where a dense id \
         is actually enumerated, on the fixture's shapes",
        dense_ids.len()
    );
    // 2. The pair COMPOSES, by type: an element of `dense_ids()` is exactly what
    //    `dense_contains` takes. Pinned as an annotation plus a coercion -- D16's instrument --
    //    because both hold with the slice EMPTY, where an iteration pins nothing at all.
    let _dense_ids_are_component_ids: Option<&ComponentId> = dense_ids.first();
    let _dense_contains: fn(&EcsMaster, Entity, ComponentId) -> bool = EcsMaster::dense_contains;
    // 3. `dense_contains` ANSWERS, on a concrete id whose answer is known independently.
    assert!(
        !ecs.dense_contains(entity, id),
        "a Table-kind dynamic tag is not a member of any dense store"
    );
    assert_eq!(
        ecs.dense_slot_of(entity, id),
        None,
        "F3 substrate: `dense_slot_of` is pub and answers None off the dense path"
    );
    assert!(
        ecs.dense_get_raw(entity, id).is_none(),
        "F3 substrate: `dense_get_raw` is pub and answers None off the dense path"
    );

    // ── per-id classification: the three §3-matrix discriminators ───────────────────────
    assert_eq!(
        storage_kind(id.get()),
        StorageKind::Table,
        "F5: `register_tag` mints a Table-kind id -- `Bitset` would put it out of every \
         signature and make the assertion on source 1 above unreachable"
    );
    assert_eq!(
        residency_class(id.get()),
        ResidencyKind::Cpu,
        "the TableGpu row of §3's matrix is reached through this accessor; a dynamic tag is \
         the Cpu control"
    );
    let layout = get_layout(id.get())
        .expect("F20: `register_tag` registers a ComponentLayout, and its `type_name` is the \
                 interned tag name -- `display_name`'s whole implementation at EG1");
    assert_eq!(
        layout.type_name, "eg0_seam_census_subject",
        "F20: a dynamic tag's ComponentLayout::type_name IS the interned name, which is what \
         makes `TableOpaque` displayable and what EG3's dynamic-tag write path keys on"
    );
    assert_eq!(layout.size, 0, "a dynamic tag is a ZST");

    // ── byte access: F1 / F2 / the change-tick read ─────────────────────────────────────
    // A ZST column answers `Some` with a dangling-but-aligned pointer; the census asserts
    // only that the three accessors RESOLVE and agree about presence. EG4/EG5 are where the
    // bytes behind them mean something.
    assert!(
        ecs.get_component_raw(entity, id).is_some(),
        "F1: the read-side raw projection resolves for an id in the entity's signature"
    );
    assert!(
        ecs.get_component_changed_tick(entity, id).is_some(),
        "F14/F15: the by-id change-tick READ exists -- S3 is the write twin it lacks"
    );
    assert!(
        ecs.get_component_raw_mut(entity, id).is_some(),
        "F2: the write-capable twin resolves for the same id"
    );

    // ── the dynamic-tag presence WRITE path, end to end, with zero new API ──────────────
    ecs.remove_tag(entity, tag);
    let after = ecs
        .entity_archetype_id(entity)
        .and_then(|a| ecs.archetype_master().get_archetype(a))
        .map(|a| a.component_ids().contains(&id))
        .expect("invariant: the entity is still live after a detach");
    assert!(
        !after,
        "§4's `set_presence`-on-a-dynamic-tag bullet, compiled: display_name -> tag_by_name -> \
         remove_tag needs NOTHING added to `boyko_ecs`. This is the bullet that survived F27 \
         while its bitset twin did not"
    );

    // ── the bitset route: SIGNATURE-pinned, never called (D16) ──────────────────────────
    // See the header. These three coercions fail to compile if the item is not public or its
    // signature moves; they mint no tag, so they cannot launder the missing half — the
    // `ComponentId` -> `EnableTagId` direction — into a green.
    let _is_enabled_id: fn(&EcsMaster, Entity, EnableTagId) -> bool = EcsMaster::is_enabled_id;
    let _enable_id: fn(&mut EcsMaster, Entity, EnableTagId) = EcsMaster::enable_id;
    let _disable_id: fn(&mut EcsMaster, Entity, EnableTagId) = EcsMaster::disable_id;
}

// ═══════════════════════════════════════════════════════════════════════════════════════
// The plan is the source of truth for the not-yet-reachable list — parsed, not copied
// ═══════════════════════════════════════════════════════════════════════════════════════

/// This rung's own plan, relative to `crates/boyko_reflect`.
const PLAN: &str = "../../docs/REFLECTION-PLAN-ECS.md";

/// The `compile_fail` corpus directory, relative to `crates/boyko_reflect/tests`.
///
/// ⚠️ **This is the ONLY spelling of the directory in this file, and that is the repair.** It
/// used to be counted here by [`fixture_paths`] and spelled again as a literal inside
/// `t.compile_fail("tests/seam_compile_fail/*.rs")` — one constant, one reader, and the reader
/// was not the glob. See header item 2 for the measurement; the glob is now
/// `format!("tests/{CORPUS}/*.rs")`, written inline at the one call site.
const CORPUS: &str = "seam_compile_fail";

/// Repository-relative path resolved against this crate's manifest directory
/// (the `vg_r0d_census.rs` / `vg_thresholds::repo_path` shape).
fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// Reads the plan. A missing plan is a **failure**, never a skip: a census that cannot find
/// its subject looks exactly like a census that scanned it and found nothing.
fn read_plan() -> String {
    let path = repo_path(PLAN);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("EG0's own plan must be readable at {}: {e}", path.display()))
}

/// One row of the not-yet-reachable list, as the plan states it.
struct SeamItem {
    /// The `Type::fn` spelling, e.g. `EcsMaster::add_component_by_id`.
    path: String,
    /// `true` for the four §4 must **add** (S1, S2, S3, S4′) — their fixtures flip to `pass`
    /// at EG2. `false` for the one item the plan **refuses** to add — its fixture is red
    /// forever. A single undifferentiated list is the defect this field exists to prevent.
    flips_at_eg2: bool,
}

/// Everything between `start` and the next `\n### ` (or EOF).
fn section_after<'a>(doc: &'a str, start: &str) -> &'a str {
    let from = doc
        .find(start)
        .unwrap_or_else(|| panic!("the plan no longer contains the heading `{start}`"));
    let rest = &doc[from + start.len()..];
    match rest.find("\n### ") {
        Some(end) => &rest[..end],
        None => rest,
    }
}

/// The first `` `…` `` span at or after `from`, truncated at the first `(` — i.e. the
/// `Type::fn` head of a signature written in backticks.
fn first_backticked_path(hay: &str, from: usize) -> Option<String> {
    let open = hay[from..].find('`')? + from + 1;
    let close = hay[open..].find('`')? + open;
    let span = &hay[open..close];
    let head = span.split('(').next()?.trim();
    if head.contains("::") { Some(head.to_string()) } else { None }
}

/// The plan's not-yet-reachable list, **derived from the document**.
///
/// The four positives come from §4's seam table — a row whose first cell is an `S`-label,
/// with its new-item signature in the second cell. The one negative comes from EG0's own
/// paragraph, the backticked name after *"refuses** to add"*.
///
/// # Non-vacuity
///
/// A parser that matches nothing hands back an empty list, and every assertion downstream
/// then passes over zero items. §4's table has **four** `S`-rows and EG0 refuses **one**
/// item, so those are the floors, asserted here rather than downstream: if the document is
/// restructured so this parse stops seeing it, EG0 reds *here*, naming the parse.
fn seam_items(doc: &str) -> Vec<SeamItem> {
    let mut out = Vec::new();

    // ── the positives: §4's seam table ──────────────────────────────────────────────────
    for line in doc.lines() {
        let Some(body) = line.strip_prefix('|') else { continue };
        let mut cells = body.split('|');
        let Some(label_cell) = cells.next() else { continue };
        // `| S1 |` and `| ~~S4~~ **S4′** |` both reduce to their LAST token once markdown
        // emphasis is stripped; anything else is not a seam row.
        let stripped: String = label_cell.chars().filter(|c| *c != '*' && *c != '~').collect();
        let Some(label) = stripped.split_whitespace().next_back() else { continue };
        let is_seam_label = label.starts_with('S')
            && label.len() >= 2
            && label[1..].trim_end_matches('\u{2032}').chars().all(|c| c.is_ascii_digit())
            && !label[1..].trim_end_matches('\u{2032}').is_empty();
        if !is_seam_label {
            continue;
        }
        let Some(sig_cell) = cells.next() else { continue };
        let Some(path) = first_backticked_path(sig_cell, 0) else { continue };
        out.push(SeamItem { path, flips_at_eg2: true });
    }
    assert!(
        out.len() >= 4,
        "PARSE FLOOR: §4's seam table yielded {} row(s), and the plan states FOUR items in one \
         owner call (S1, S2, S3, S4′). An empty or short parse makes every assertion below \
         vacuous, so it reds here instead.",
        out.len()
    );

    // ── the negative: EG0's own paragraph ───────────────────────────────────────────────
    let eg0 = section_after(doc, "### EG0 —");
    let refuses = eg0
        .find("refuses")
        .expect("PARSE FLOOR: EG0's paragraph must still say what this plan REFUSES to add -- \
                 the two-kind distinction is what stopped one item sitting on the negative list \
                 while a sibling plan declared it mandatory");
    let refused = first_backticked_path(eg0, refuses).expect(
        "PARSE FLOOR: the refused item must be named as a backticked `Type::fn` right after \
         EG0's `refuses to add`",
    );
    out.push(SeamItem { path: refused, flips_at_eg2: false });

    let refused_count = out.iter().filter(|i| !i.flips_at_eg2).count();
    assert_eq!(
        refused_count, 1,
        "PARSE FLOOR: EG0 refuses exactly ONE item (`TagId::from_component_id`); {refused_count} \
         were parsed"
    );
    out
}

/// Every `.rs` fixture in the corpus. The `fixture_count(dir)` shape
/// `crates/reflect_fixture/tests/reflect_compile_fail.rs:91` already declares and which each
/// of its four legs asserts against — the one *local* instrument that catches an empty corpus,
/// because `running N` cannot.
///
/// ⚠️ It only catches it because it counts [`CORPUS`], **the same string the glob is built
/// from**. The shape it was copied from spelled the directory twice, so its floor and its glob
/// could name different places and the floor guarded nothing — measured, and repaired on both
/// sides. The class-level backstop is `tests/trybuild_corpus_compiler_witness.rs`, which
/// resolves every `trybuild` glob in the tree and reds on one that matches no fixture.
fn fixture_paths() -> Vec<PathBuf> {
    let dir = repo_path("tests").join(CORPUS);
    let mut out: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("the seam corpus must exist at {}: {e}", dir.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .collect();
    // `read_dir` order is platform-defined; the report a failure prints must not be.
    out.sort();
    out
}

/// Every fixture paired with the blessed `.stderr` beside it — the compiler's own word about
/// the item, which is what the correspondence below binds against.
fn corpus() -> Vec<(PathBuf, String)> {
    fixture_paths()
        .into_iter()
        .map(|path| {
            let blessed = path.with_extension("stderr");
            let text = std::fs::read_to_string(&blessed).unwrap_or_else(|e| {
                panic!(
                    "every fixture needs a blessed .stderr; {} is missing one: {e}",
                    blessed.display()
                )
            });
            (path, text)
        })
        .collect()
}

/// The last two `::`-segments of a plan row's spelling — the pair rustc names when it cannot
/// resolve `Type::fn`. A row qualified with a module reduces to the type, because the type is
/// what the compiler prints.
fn type_and_fn(path: &str) -> Option<(&str, &str)> {
    let mut segments = path.rsplit("::");
    let func = segments.next()?;
    let ty = segments.next()?;
    if func.is_empty() || ty.is_empty() { None } else { Some((ty, func)) }
}

/// **The binding.** Does this blessed `.stderr` carry the one sentence rustc writes only when
/// resolution **looked for `Type::fn` and found nothing**?
///
/// `E0599` is that sentence's error code, and its wording names both halves the plan spells:
///
/// ```text
/// error[E0599]: no associated function or constant named `add_component_by_id`
///               found for struct `EcsMaster` in the current scope
/// ```
///
/// The middle noun varies (`method`, `associated function or constant`, `variant or associated
/// item`), so it is not matched; the **code**, the **function name**, the **type name** and the
/// closing *"in the current scope"* are.
///
/// ⚠️ **Substring-matching the `Type::fn` path — the first cut — is a different and much weaker
/// property, and both of its failure directions were MEASURED (header, "the two ways a
/// substring failed"):** rustc echoes the whole source line including trailing comments, so a
/// comment satisfies it; and a machine-applicable suggestion prints the call path-qualified
/// *because the item resolved*, so reachability satisfies it too. Neither produces an `E0599`
/// line naming the item, which is why that line is what is matched.
fn says_the_compiler_looked_and_did_not_find(err: &str, path: &str) -> bool {
    let Some((ty, func)) = type_and_fn(path) else { return false };
    let named = format!(" named `{func}` found for ");
    let ty_spelled = format!("`{ty}`");
    err.lines().any(|line| {
        let Some(rest) = line.strip_prefix("error[E0599]: no ") else { return false };
        let Some((_kind, tail)) = rest.split_once(named.as_str()) else { return false };
        tail.contains(&ty_spelled) && tail.trim_end().ends_with("in the current scope")
    })
}

/// **EG0 gates 2 + 3 + 4.** The plan's not-yet-reachable list and this corpus name the *same*
/// items, one fixture each, and the compiler is the witness for every row.
///
/// The correspondence is asserted against the **blessed `.stderr`**, not the filename and not
/// a comment — via [`says_the_compiler_looked_and_did_not_find`], which looks for `E0599` over
/// the plan's own `Type::fn`. Nothing but a failed resolution produces that line, so neither a
/// comment naming the item nor a suggestion produced by the item *existing* can satisfy it.
/// Both of those were measured against the first cut; the header records them.
///
/// When EG2 lands an item, this test is the alarm: the fixture flips to `pass`, its `.stderr`
/// goes away, and the row must move off §4's not-yet-reachable list in the same commit. If an
/// implementer instead re-blesses a fixture whose item has become reachable — the `E0061`
/// shape the stub produced — this test reds rather than certifying the opposite of its claim.
#[test]
fn the_plan_and_the_corpus_name_the_same_not_yet_reachable_items() {
    let doc = read_plan();
    let items = seam_items(&doc);
    let corpus = corpus();

    assert_eq!(
        corpus.len(),
        items.len(),
        "the plan names {} not-yet-reachable item(s) and the corpus holds {} fixture(s). \
         UP means a fixture nobody's plan row asks for; DOWN means a plan row with no \
         compiler behind it -- and DOWN is the one that reads as green everywhere else.",
        items.len(),
        corpus.len()
    );

    let witness = |item: &SeamItem| -> Vec<PathBuf> {
        corpus
            .iter()
            .filter(|(_, err)| says_the_compiler_looked_and_did_not_find(err, &item.path))
            .map(|(p, _)| p.clone())
            .collect()
    };

    for item in &items {
        let matches = witness(item);
        assert_eq!(
            matches.len(),
            1,
            "`{}` must be named by EXACTLY ONE blessed .stderr as an E0599 -- the sentence rustc \
             writes only when resolution LOOKED for it and found nothing; {} matched. If nothing \
             matched, either the fixture calls it unqualified (a filename or a comment is NOT the \
             binding) or the item BECAME REACHABLE and its diagnostic is now some other code -- \
             `E0061` for a signature mismatch, `E0624` for a private one -- in which case the row \
             must move off the plan's list in the same commit. Matching the path as a substring \
             would have passed in BOTH of those cases; that is measured, and it is why this \
             matches the diagnostic and not the spelling.",
            item.path,
            matches.len()
        );
    }

    // ── the two kinds, ASSERTED apart rather than logged apart ──────────────────────────
    // `flips_at_eg2` is parsed from two DIFFERENT places in the document -- §4's seam table
    // for the four that flip, EG0's own `refuses to add` sentence for the one that never does
    // -- so "the same item on both lists" is a reachable state, and it is the exact state EG0
    // and BOUNDARY's D10 were in until F27 forced the question. Until this assertion the field
    // carried one parse floor and a `println!` label: nothing over the corpus treated the kinds
    // differently, so the distinction the header calls load-bearing bore no load.
    let refused: Vec<&SeamItem> = items.iter().filter(|i| !i.flips_at_eg2).collect();
    let flipping: Vec<&SeamItem> = items.iter().filter(|i| i.flips_at_eg2).collect();
    assert!(
        !refused.is_empty() && !flipping.is_empty(),
        "both kinds must be non-empty or the cross-check below is vacuous: {} refused, {} \
         flipping",
        refused.len(),
        flipping.len()
    );
    for r in &refused {
        for f in &flipping {
            assert_ne!(
                r.path, f.path,
                "`{}` is on BOTH kinds of the not-yet-reachable list: §4's seam table says EG2 \
                 must ADD it, and EG0's paragraph says this plan REFUSES to. One of the two \
                 sections is wrong, and an undifferentiated negative list is what let exactly \
                 this state survive between two sibling plans until F27.",
                r.path
            );
            assert_ne!(
                witness(r),
                witness(f),
                "the refused item `{}` and the flipping item `{}` are certified by the SAME \
                 fixture. The per-item count above cannot see this -- a single .stderr naming \
                 both still matches each of them exactly once -- and D21 records that the seam \
                 items DO contaminate each other's blessed output. A refusal that never gets \
                 its own compiler run is a row with no witness.",
                r.path,
                f.path
            );
        }
    }

    // Print the census, so a reader of the log sees WHAT was compiled, not only that
    // something was. Two kinds, and the distinction is now asserted above, not only labelled.
    for item in &items {
        let kind = if item.flips_at_eg2 { "flips to pass at EG2" } else { "refused forever" };
        println!("EG0 seam census: `{}` -- not reachable today, {kind}", item.path);
    }
}

// ═══════════════════════════════════════════════════════════════════════════════════════
// The negative half — the trybuild corpus
// ═══════════════════════════════════════════════════════════════════════════════════════

/// **EG0 gate 2.** Every item on the not-yet-reachable list fails to compile, with its
/// blessed message.
///
/// The floor is **read from the plan** rather than written here as a literal, so the two
/// cannot drift and the number is derived rather than asserted. `>=`, not `==`: adding a
/// fixture must never red a count, and deleting the last one must — the exact-correspondence
/// assertion lives in
/// [`the_plan_and_the_corpus_name_the_same_not_yet_reachable_items`].
///
/// **An empty glob is a VACUOUS PASS and it was MEASURED in this package**: `trybuild` prints
/// *"There are no trybuild tests enabled yet"*, the harness reports `running 1 test … ok`,
/// and the process exits **0**. `running N` does not catch it — the harness function runs and
/// passes over zero fixtures.
///
/// ⚠️ **A floor guards the glob only if it counts the SAME directory the glob compiles**, and
/// this one did not until the EG0 verification: the glob was a second, independent literal, so
/// mutating one character of it emptied the run at exit **0** with the floor untouched. It is
/// derived from [`CORPUS`] now. `tests/c6_compile_fail.rs`, the nearest template, carried **no** floor
/// at all — the vacuous shape an implementer inherits by proximity (D18) — and has one now.
#[test]
fn the_not_yet_reachable_seam_items_still_do_not_compile() {
    let doc = read_plan();
    let floor = seam_items(&doc).len();
    let n = fixture_paths().len();
    assert!(
        n >= floor,
        "the seam corpus holds {n} fixture(s) and the plan names {floor} not-yet-reachable \
         item(s). An empty or short corpus is a VACUOUS PASS in this harness -- measured."
    );

    let t = trybuild::TestCases::new();
    t.compile_fail(format!("tests/{CORPUS}/*.rs"));
}
