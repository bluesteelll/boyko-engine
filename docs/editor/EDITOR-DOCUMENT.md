# E6 — The document: what the editor edits, and where Gaia's territory begins

**Part of the editor campaign design.** Index and v1 ladder: `EDITOR-DESIGN.md`. **Revision 5.**
**Revision 5** re-tensed §1's storage-walk-vs-snapshot-walk contrast: the `serialize_ui` loss it
cited was repaired 2026-09-03, so the contrast is now a claim about SHAPES with that history as its
measurement, and §1 says what the repair cost and why the editor's choice is unaffected.
**Revision 4** replaced G-B's oracle (§4) — the type-table census could not fail — and named the one hole the two gates still leave open.
Decisions `E6.1`–`E6.2`. Code claims are read at HEAD `98ede3e0`; the ones revision 5 added are read
in the WORKING TREE at HEAD `59009f8a`.

| § | decision |
|---|---|
| §1 | `E6.1` — what the editor edits, and its persistent form |
| §2 | `E6.2` — the Gaia boundary |
| §3 | keeping the editor out of the document — `EK3` |
| §4 | the gate — `V4`'s round trip |
| §5 | alternatives, priced |
| §6 | out of scope, and what this file did NOT establish |

---

## 1. What the editor edits — `E6.1`

**Decision. The document IS the edit World. There is no retained document model beside it. Its
persistent form in v1 is the `boyko_serialize` binary written by `save_world` with the `EK3`
exclusion.**

Choosing the World as the document rather than a tree is Principle 0, and it has a measured payoff
rather than an aesthetic one: **a saver that walks the STORAGE cannot lose a component it does not
know about, while a saver that walks a hand-written snapshot loses every component nobody remembered
to add.** The engine contains one of each. `save_world` iterates
`world.archetype_master().iter_archetypes()` and, within each, every `component_id` in the
archetype's signature (`crates/boyko_serialize/src/save.rs:170-190`) — it cannot omit a component
type by forgetting it, only by that type being classified `Ignore`. `serialize_ui` walks a
hand-written `LiveNode` snapshot, and until 2026-09-03 it wrote 8 of the 20 non-exempt components its
own parser accepts (`crates/boyko_ui/src/text/serialize.rs:21-23`, HEAD `59009f8a`), with a green
round-trip gate over the loss. The editor takes the first shape.

**What the repair of that second saver changed here, stated honestly.** The `.ui` writer no longer
loses those twelve: the vocabulary became ONE declaration
(`crates/boyko_ui/src/text/vocab.rs:163-205`), every consumer matches on it exhaustively, `LiveNode`
now carries the whole vocabulary (`crates/boyko_ui/src/reload/tree_view.rs:48-87`) and a world-to-
world gate re-reads it (`crates/boyko_ui/tests/p3_world_round_trip.rs`). So the sentence in bold can
no longer be read as "and one of the two is broken right now". It survives as a claim about SHAPES,
with the history as its measurement: the snapshot-walker lost twelve components for as long as
nobody re-read its list, and repairing it cost a declaration, three exhaustive matches and two
behavioural gates — none of which the storage-walker needs, because it has no list to keep in step.
The two shapes are not interchangeable, though, and the difference cuts the editor's way rather than
against it: `.ui` MUST have a closed vocabulary (a text file may only construct UI components — a
structural safety property for untrusted, hand-edited text), so a declaration is unavoidable there,
while a world saver must NOT have one, and `E6.1`'s document is a world.

**What the binary form buys and costs.**

| buys | costs |
|---|---|
| Zero format work: it exists, it is tested (S1/S1.5/S2/S2.5), and it is the format the SHIPPED loader reads | no diff, no merge, no text review — a document is an opaque blob to `git` |
| POB columns blit; the cost scales with scene bytes rather than entity count | resources are not in the format; a document is entities and their components |
| Entity remap on load is done (S2.5): `ChildOf` and `#[entities]` fields are rewritten to the freshly-allocated `Entity`, and an unmapped saved id is a loud `LoadError`, never a silent dangling reference | a plain `Entity` field WITHOUT `#[entities]` is deliberately not remapped (the C4 opt-in decision), so it survives as a stale raw id |

The diff/merge/review cost is real and is owner question 3 — it is a VALUE, not an architecture fork.
The alternative (making `V4` depend on Gaia's text form) is priced in §2.3.

---

## 2. The Gaia boundary — `E6.2`

### 2.1 What is already ruled, and why it is favourable

The boundary is decided on paper and in this campaign's favour: **Gaia's baker prints the existing
`boyko_serialize` format rather than minting a second one**
(`docs/AETHER-GAIA-REVISION-2026-08-29.md`, shared-surface boundary). So there is ONE runtime format,
the editor is not waiting on a format decision, and the editor's v1 binary path is a stepping stone
rather than throwaway work.

Two more rulings bind:

- **`F1`, ruled by the owner 2026-08-30: the bake route is macro-time `GK-4`, taken NOW** — not
  sequenced behind the refused `EG2` reflection seam.
- **`F6`: `.ui` migrates into Gaia's `ui` profile and is DELETED in the same campaign** — which is one
  of the three reasons the editor's chrome is not authored in `.ui` (`EDITOR-UI.md` §4).

### 2.2 The division of labour

| owns | Gaia | the editor |
|---|---|---|
| the text GRAMMAR | ✔ | — |
| the BAKER (text → `boyko_serialize` bytes) | ✔ | — |
| the cross-load id map (`GK-1`) | ✔ | consumes it (`EDITOR-BOUNDARY.md` §5); lands it under `GK-1`'s name if `G6` has not |
| the component FIELD tables | ✔ (`GK-4`, at `G1`) | consumes them; and lands the TYPE they should use (`EK4`/`EK7`, §2.3) |
| composition / templates / streaming | ✔ | — |
| the `ui` profile | ✔ | — |
| the runtime BINARY format | already exists (`boyko_serialize`) | writes it |
| **the EDIT operations** — what a user or an agent does to a document, and its inverse | — | ✔ (`EDITOR-COMMANDS.md`) |
| **the exclusion of editor state from the document** | — | ✔ (`EK3`, §3) |

The editor mints **no format, no id namespace and no grammar**. Where it needs something a Gaia rung
owns, it files a requirement against that rung rather than building a parallel one — the failure this
repository has recorded as discovering late that a sibling plan already owns your rung.

It files exactly one: **`EK7`** — when `G1` lands `GK-4`, it emits `EK4`'s `FieldTable` rather than
minting a second descriptor.

### 2.3 The sequencing, honestly — and the correction revision 3 makes

**No rung of the v1 ladder waits on the Gaia campaign.** Revision 2 said `V5` alone did (because the
inspector needed `GK-4`'s field tables) and, worse, was wrong about its own ladder: `CommandDesc`
embedded `GK-4`'s tables, so `V1` and `V3` stood on the same unstarted rung.

Revision 3 removes the dependency at its root. `EK4` lands `FieldTable` as a KERNEL type with two
emitters — `#[derive(CommandArgs)]` for command argument and result structs, and an opt-in
`#[derive(Fields)]` for components (the `#[derive(Bindable)]` precedent, already in the tree and
documented as reflection-free). The full argument is in `EDITOR-COMMANDS.md` §3; the part that
belongs here is the boundary consequence:

- The editor does not wait on Gaia, at any rung.
- Gaia does not inherit a competing descriptor: `EK4`'s shape omits exactly the column ballot `GB-5`
  gates — Gaia's own campaign row says "the table may be designed and emitted; the **disposition
  column for engine-derived fields may not be minted** until `GB-5` is answered" — so `G1` EXTENDS
  `FieldTable` rather than replacing it, and `#[derive(Fields)]` being opt-in means `G1` widening it
  to every component under its own gate is additive.
- The transition to Gaia TEXT is additive and does not touch the editor's surface: **the editor's
  command names and argument tables do not change across it.** `document.save` switches from calling
  `save_world` to calling Gaia's canonical printer; the document becomes reviewable; nothing above
  moves.

The editor is also the tool `AIR-09` names — the one that applies `{node_id, key, value}` edits and
canonically re-prints — so when Gaia's text form lands, the editor is its intended writer rather than
a second one.

**The price of NOT waiting, stated:** v1 documents are opaque binaries (§1). The price of waiting:
`V4` would depend on an unstarted campaign (four `.md` files, no crate, no workspace member, `G0`–`G8`
unstarted) whose language spec is itself marked ratified-stale with an owner-gated rewrite pending.
Owner question 3.

---

## 3. Keeping the editor out of the document — `EK3`

Full rule and rationale: `EDITOR-BOUNDARY.md` §3. Summarised here because it is the document's
property, not only the boundary's:

- **v1 attaches NOTHING to a document entity.** Selection is an editor-owned `Selection` resource
  keyed by `StableId`; lock and hide are cut. So the ordinary case — open, select, save — changes no
  archetype and writes no extra byte.
- **Editor-OWNED entities** (gizmos, panel roots, chrome) carry `#[component(no_serialize)]` and
  `#[require(EditorOnly)]`, and `EK3` excludes them in TWO arms: an archetype-signature arm and a
  DENSE arm, because the dense pass is keyed by entity rather than by archetype ("a dense store's
  slot→entity map is NOT the archetype entity list", `crates/boyko_serialize/src/save.rs:277-283`).
- **The exclusion is opt-OUT, not structural.** `#[derive(Component)]` classifies `PlainOldBytes` for
  an all-bits-valid `#[repr(C)]` type and `SerializeViaFn` for any other `Clone` type
  (`crates/boyko_ecs/src/ecs/core/component/component.rs:161-166`); the `Ignore` default belongs to a
  hand-written `impl Component`, which no editor component will be. A forgotten
  `#[component(no_serialize)]` produces no error at all — the file simply gains a column nobody reads
  until a loader in a shipping build meets it. That is why gate G-C exists, and why this design does
  not claim the mechanism is "Godot's `owner` by another route": `owner` is structural and
  unforgeable; this is an attribute a developer can omit.

---

## 4. The gate — `V4`'s round trip

Revision 2's gate was a save→select→save byte compare. **It is replaced, for two independent
reasons**, and the replacement is the more important half of this file.

1. **It was unsatisfiable** under a Table-storage annotation: attaching one migrates the entity into
   a different archetype, and `save_world` emits one block per archetype carrying that archetype's
   own entity-id array (`save.rs:85-92,170-175`), so the block sequence and the intra-block ordering
   change even though no datum is lost. An implementer would meet a red that is not a defect and
   redefine the oracle at the bench. (Revision 3 also removes the CAUSE, §3, but a gate that only
   works because the case does not arise is not a gate.)
2. **A save-vs-save byte compare is blind in the direction that matters.** Both sides come from the
   same saver, so both omit the same thing. This is the `.ui` lesson restated — `serialize → parse →
   serialize` is byte-identical when both sides are equally lossy — and it recurred inside a document
   that cites that lesson two sections later.

**The replacement splits the two properties the old gate conflated**, and gives each an oracle that
survives archetype fragmentation:

| gate | property | oracle |
|---|---|---|
| **G-A** | nothing is LOST | open the fixture, spawn chrome, select, `document.save`; load the saved file into a FRESH world; compare against the FIXTURE with `EK16`'s order-independent, `StableId`-keyed digest over the full serializable set — entity SET, per-entity component set, per-component bytes. Order-independent by construction (`EDITOR-REPLAY.md` §1.5), so archetype order, row order and the load's id remap cannot make it red for a non-reason |
| **G-B** | nothing EXTRA is written | **(revision 4)** an entity-id SET census over the saved FILE: union every archetype block's entity-row array with every dense block's `s2e` array, and require set equality with the document's entity set. Reads the file's own tables — order-independent, and it catches an editor entity the fixture-keyed digest would never look for. It replaces a type-table census that could not fail: an editor component is `Ignore`, and `save_world` skips an `Ignore` column BEFORE `intern_type` (`crates/boyko_serialize/src/save.rs:174-186`), so its name is absent from the table with or without `EK3` |
| **G-C** | the convention is a mechanism | the source census over `crates/boyko_editor/**` |

Each is red-first: G-A reds when one document entity is dropped from the saver's output (naming the
missing `StableId`); G-B reds before `EK3`'s dense arm exists, because the fixture includes an editor
entity carrying a dense component; G-C reds on a derived component missing its attribute. Each
REPORTS its counts, so a gate that compared nothing cannot read as a pass.

**The two gates are complementary, not redundant.** G-A is keyed on the fixture and would not notice
an extra editor ENTITY in the file (the document entities it compares all match). G-B reads the file
and would not notice a MISSING component on a present entity. Together they cover both directions;
either alone is the shape that goes green over a defect.

**One hole is left open deliberately and named here rather than papered over.** Neither gate catches
an extra editor COLUMN attached to a document entity — G-A compares the components the fixture has,
G-B compares entity ids. In v1 that hole is unreachable by construction, because `E1.3`'s Class A set
is EMPTY: no editor component is ever attached to a document entity. The moment a Class A annotation
is introduced, G-B gains a second half (the file's type table must name no Class A `stable_name` —
which becomes a gate that CAN fail, because a Class A component is `no_serialize` + `dense` and the
dense pass has its own `Ignore` skip to get wrong at `save.rs:286-296`). Recorded so the gate grows
with the mechanism instead of after it.

---

## 5. Alternatives, priced

| alternative | price | verdict |
|---|---|---|
| **Wait for Gaia `G2` so v1 documents are text** | makes `V4` depend on an unstarted campaign whose spec is ratified-stale; buys diff/merge/review | rejected for v1; owner question 3 |
| **Mint an editor scene format** | two formats, and the ruled boundary forbids it | rejected |
| **Author the document through `.ui`** | `F6` deletes the format, and a scene is not a UI tree. (This row also read "`serialize_ui` drops 12 of 20 components with a green gate" — true until 2026-09-03, repaired since; see §1 and `EDITOR-UI.md` §4 reason 2. The two surviving prices are each sufficient) | rejected |
| **A retained document model beside the World** (Godot's `EditedScene` + `EditorData`) | it is the canonical case's answer, and it is a parallel data system — Principle 0 forbids it, and the engine's own alternative (components + a save filter) is strictly better here because the saver walks storage | rejected |
| **Per-entity save filter added to `save_world`** instead of a per-component exclusion | S in isolation; but an entity-filtered save immediately raises the dangling cross-entity reference question, and entity remap on load is `Remap`-only | rejected — the per-component rule plus two processes covers v1 without opening it |

---

## 6. Out of scope, and what this file did NOT establish

**Out of scope (recorded):** multi-document editing; a document format version migration path (the
loader is already lenient about unknown types, `LoadReport::types_skipped`); resources in the document
(the format holds entities and components; editor and engine resources are reconstructed at load);
prefabs/templates as a document concept (Gaia's composition owns it, and `EcsMaster::instantiate`
already exists for the runtime half); external asset references beyond what `boyko_serialize` already
carries; and the Gaia TEXT save path plus the id→text-range manifest consumer (`G2`–`G3`).

**NOT established:**

- **The cost of `EK16`'s digest over a real document**, which G-A runs twice per gate invocation.
  Reported at `V4`, not gated.
- **Whether `save_world`'s `include_filter` is the right hook for `EK3`'s archetype arm**, or whether
  the exclusion belongs in the archetype loop directly. The filter keys on `ComponentId` and the
  exclusion keys on the SIGNATURE containing one, which is not the same predicate; `V4` decides by
  reading, and the design states the requirement rather than the call site.
- **What `document.save` does when the world contains an entity whose `#[entities]` reference points
  at an EXCLUDED editor entity.** The loader's remap raises a loud `LoadError` on an unmapped id, so
  this would fail loudly rather than silently — which is the right failure — but it means an editor
  that ever stores a gizmo handle in a document component breaks the save. `EK9`'s census should
  forbid a document-reachable component holding an editor `Entity`, and `V4` must confirm the loud
  failure rather than assume it.
- **Whether autosave (`EDITOR-BOUNDARY.md` §4.4) can run without a frame's worth of stall** on a
  large document. It is a `save_world` on the exclusive window; `V4` reports the time and the
  cadence can be tuned, but a document large enough to hitch would need an incremental save, which is
  out of scope.
