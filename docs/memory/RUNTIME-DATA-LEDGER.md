# Runtime Data Ledger - rev 3 (index)

**What this is.** Every runtime heap site in the engine's code, one row each, with the ECS form the datum takes in the one unified system. Rev 2 splits the ledger: this index carries the vocabulary, the totals, the entity model, the kernel features, the decisions and the order of work; the thirteen group files under [`ledger/`](ledger/) carry every row in full; [`runtime-data-ledger.tsv`](runtime-data-ledger.tsv) is the machine-readable row set.

**Trees.**

| groups | tree | branch @ commit | note |
|---|---|---|---|
| the eleven rev-1 groups | `D:/wt/joltab` | `merge/ke16-into-ecsnative` @ `d11962a9` | crates/*/src identical to `ca582e72`, where rev 1 was taken. The working copy has uncommitted edits in 15 boyko_ecs files; the 110 citations into those files hold at the commit, not in the working copy. |
| ui-lane | `D:/wt/ui` | `feat/ui-advanced` @ `615cda8f` | the census of record for crates/boyko_ui/src (rev 2) and, since rev 3, for every non-test site the lane adds or changes in other crates (196 rows) |
| reflect-lane | `D:/wt/reflect` | `feat/reflection` @ `0e0b4c68` | rows only in code the lane added (24 rows) |

**Date:** 2026-09-11. **Revision:** 3. **Status:** complete ledger, not yet a gate (section [What this ledger is not](#what-this-ledger-is-not)).

**How rev 3 was made.** Read-only on every code tree: python, git show, git diff. No cargo, clippy or test. Input: rev 2 (below) and its recheck, whose seven work items and six minor items are the whole scope of rev 3. Every change is one record `{item, tsv_key, field, old, new, evidence}` in `rev3/changes.json`; the fixer is `rev3/synth3/build3.py`, the renderer `gen3.py`, the checker `verify3.py` (section [Change log](#change-log), Rev 2 -> rev 3). Decisions follow the owner's delegation (performance first) and the two decisions files.

**How rev 2 was made.** Read-only on every code tree: python, git show, rg. No cargo, clippy or test. Inputs:

1. the rev-1 group ledgers with the nine gaps of the completeness critic closed (`rev2/<group>.ecsform.json`, 1209 logged field changes, 2193 -> 2223 rows);
2. two lane censuses, ui-lane and reflect-lane;
3. two defect verdicts (section [Defects](#defects));
4. the writer's reconciliations W1-W8, which make the three inputs agree with each other and with the physics and engine decisions files (section [Change log](#change-log)).

The working files (the group JSON ledgers with every non_row, the lane JSONs, `gap_changes.json`, and the writer's scripts and log under `synth2/`: `build_rev2.py`, `gen2.py`, `verify2.py`, `writer_changes.json`) are in the session scratchpad, `C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/dad9a19e-5512-4bbd-9976-cb44400eeb64/scratchpad/ledger/rev2/`, and are not committed.

**Disclosure:** the gap agent ran `rustc.exe --version` once, by mistake, while locating rust-src; it built nothing. The rev-1 disclosures (two `rustc --version` runs in the timing window) stand in the pool-utils-log and app-demo files.

**Totals:** **2350 active rows** in 23 crates from 13 groups; 131 superseded rows (122 kept in ui-input.md, 9 in render.md); 4492 non_rows (rev 3: no supplementary rows, and the non_rows of sites a lane re-censused are not counted twice). **673 rows (28.6 %) land in an ECS data form** (component, dense-component, enable-state, relation, event, resource-column or system-scratch). 264 are kernel-internal, 301 diagnostics, 10 scope-arena, and 1102 out of scope, 1021 of them compile-time. **0 rows are class U. 0 questions are left undecided.**

## Contents

- [The owner's orders](#the-owners-orders)
- [Vocabularies](#vocabularies)
- [Totals](#totals)
- [The entity model](#the-entity-model)
- [Kernel features](#kernel-features)
- [Primitives refuted](#primitives-refuted)
- [Plan conflicts](#plan-conflicts)
- [Order of work](#order-of-work)
- [Decisions (formerly undecided)](#decisions-formerly-undecided)
- [Defects](#defects)
- [Change log](#change-log)
- [What this ledger is not](#what-this-ledger-is-not)
- **Rows by group** (one file each, every row in full):
  - [ecs-storage](ledger/ecs-storage.md): 109 active rows (D:/wt/joltab)
  - [ecs-schedule](ledger/ecs-schedule.md): 166 active rows (D:/wt/joltab)
  - [ecs-services](ledger/ecs-services.md): 77 active rows (D:/wt/joltab)
  - [pool-utils-log](ledger/pool-utils-log.md): 131 active rows (D:/wt/joltab)
  - [physics-scene-math](ledger/physics-scene-math.md): 95 active rows (D:/wt/joltab)
  - [render](ledger/render.md): 160 active rows, 9 superseded (D:/wt/joltab)
  - [rhi](ledger/rhi.md): 64 active rows (D:/wt/joltab)
  - [ui-input](ledger/ui-input.md): 83 active rows, 122 superseded (D:/wt/joltab)
  - [app-demo](ledger/app-demo.md): 256 active rows (D:/wt/joltab)
  - [codec-tools](ledger/codec-tools.md): 527 active rows (D:/wt/joltab)
  - [macros-aether](ledger/macros-aether.md): 462 active rows (D:/wt/joltab)
  - [ui-lane](ledger/ui-lane.md): 196 active rows (D:/wt/ui)
  - [reflect-lane](ledger/reflect-lane.md): 24 active rows (D:/wt/reflect)

## The owner's orders

**Order 1 (2026-09-10, verbatim in translation):** "Forbid Vec by a gate. There is not one reason to use an allocator other than ours. If something is missing, extend the memory library." and then: "And the task is also: move ALL runtime data structures onto our system, and all arrays etc. onto the ECS."

**Order 2 (2026-09-10 23:15, verbatim in translation):** "The point is not only to move everything onto our allocator, but to bring everything as close as possible to the ECS PARADIGM - in particular in PHYSICS - and to make ONE UNIFIED SYSTEM."

So a row is **not finished** when its bytes come from the memory library. It is finished when the datum lives in the ECS in its natural ECS form and ECS systems on the engine scheduler read and write it. The standing rules behind this are CLAUDE.md principle 0 and the owner's earlier rulings:

- `boyko_ecs` is THE SDK for data AND logic.
- There is no parallel data system.
- A capability that a subsystem needs becomes a FIRST-CLASS KERNEL FEATURE that every crate uses the same way, never a crate-local wrapper.
- The owner has already rejected a bespoke VmReservation-backed column primitive for physics in favour of the real `ComponentPool`. Any new memory primitive standing beside `ComponentPool` is therefore presumed wrong until every ECS form has been tried and refuted with evidence.

**Order 3 (2026-09-11, verbatim in translation):** "Decide all the questions yourself, whichever is best for performance." (main:docs/unification/ENGINE-RUNTIME-ECS-DECISIONS.md:10 "The owner, 2026-09-11, verbatim in translation:".) So every place rev 1 said "owner decision" is decided in rev 2, on the throughput of the frame, and each decision names the measurement or gate that would overturn it (section Decisions). Where two options cost the same on every hot path, the tie-break is the owner's standing goal: one unified ECS system. The orders cover every runtime subsystem, not only physics: the UI is a first-class part of the one engine (the ui-lane group is its census: crates/boyko_ui/src since rev 2, and since rev 3 every non-test site the lane adds or changes in any other crate).

**Decisions already taken elsewhere under the same delegation, and followed here:**

- Physics (main:docs/physics/PHYSICS-ECS-UNIFICATION-DECISIONS.md:3 "- **Date:** 2026-09-11"): Q1 contacts are kernel events (main:docs/physics/PHYSICS-ECS-UNIFICATION-DECISIONS.md:29 "| Q1 | How gameplay sees contacts | **(a) Kernel events only**: contact and sensor enter/exit |"); Q2 soft-body particles go in the K7 segmented dense column (main:docs/physics/PHYSICS-ECS-UNIFICATION-DECISIONS.md:30 "| Q2 | Soft-body particle storage | **(a) K7, the segmented dense column** |"); Q3 sleep lives in `BodyGate` in the solver group plus transition events (main:docs/physics/PHYSICS-ECS-UNIFICATION-DECISIONS.md:31 "| Q3 | How gameplay sees sleep | **(a) `BodyGate` in the group, plus sleep/wake transition events** |"); Q5 `RigidBody` stays the authored table component and the solver works on the derived dense group (main:docs/physics/PHYSICS-ECS-UNIFICATION-DECISIONS.md:33 "| Q5 | Where pose and velocity live | **(a) `RigidBody` stays an authored table component; the solver works on the").
- Engine (main:docs/unification/ENGINE-RUNTIME-ECS-DECISIONS.md:3 "- **Date:** 2026-09-11"): Q1 assets are entities (main:docs/unification/ENGINE-RUNTIME-ECS-DECISIONS.md:23 "| Q1 | Assets as entities | **(a) Assets are entities** | yes |"); Q3 window and player are entities now (main:docs/unification/ENGINE-RUNTIME-ECS-DECISIONS.md:25 "| Q3 | Multiplicity in v1 | **(b) Window and player are entities now** | no - the design recommended (a) |"); Q4 the dead paths are deleted (main:docs/unification/ENGINE-RUNTIME-ECS-DECISIONS.md:26 "| Q4 | Delete dead paths (`Gpu3dInstance` + `Render3dPlugin`, `TelemetryStream`, legacy").

**Why this is unification and not speed.** These are the brief's measurements, taken 2026-09-10 on this machine. This ledger did not re-measure them.

- **Heap A/B.** On the 1240-body parallel pile, the Windows system heap against mimalloc gave mi/sys = 0.998 / 1.023 / 0.992 at W = 1 / 8 / 16. The band is ±5 % at W8, from 4 clean passes. A reversed-order re-run at W8 gave 1.005.
- **Allocations per step on the shipped KE16 pool.** This tree has Stage 3b: scoped task cells are placed in the per-scope `ScopeBlock`. A probe against `D:/wt/joltab`'s own prebuilt rlibs on the Jolt pyramid measured:
  - 2.1 / 331.6 / 331.6 allocations per step at W = 1 / 8 / 16 (max 339);
  - 1.27 MB per step at W8 (each scope takes at least one 4 KiB chunk);
  - 0.1 cross-thread frees per step and 0 reallocs.
- **Where the 2671-per-step figure came from.** It was measured on `D:/wt/ecsnative @ ad0ebea4`, whose pool predates Stage 3b (`d51b4ced`). It does not describe this tree.

**Conclusion: heap acquisitions are not a measurable speed lever here.** They are removed to get one unified system. The Jolt residual (2.5x at W8) is consistent with about 45 % serial work:

- a serial narrowphase system;
- a serial broadphase below 4096 bodies;
- colours under 256 slots solved inline.

The ECS answer is to run the stages as parallel systems over dense kernel columns, with intra-system parallelism as a kernel feature. That is also the performance answer (see rung 3 of the Order of work).

## Vocabularies

### Classes (lifetime / ownership, exactly one per row)

| class | name | meaning and destination |
|---|---|---|
| K | kernel-storage implementation | The ECS's own storage bookkeeping: free lists, slot maps, live bitmaps, archetype registries. Not exempt: it moves onto VmReservation, as DenseStore's s2e already did. |
| E | per-entity / per-element durable data | Bodies, contacts, instances, glyphs, widgets. Goes to a ComponentPool column or a dense component. |
| R | resource-owned bulk table | Registries, LUTs, palettes, command and event queues. |
| F | per-frame transient | Built and drained every frame. |
| S | per-scope task transient | Spawn cells, scope shared state, join lists. Goes to the ScopeBlock arena (block.rs). |
| B | setup / load-path only | Grows once at boot or asset load and is never touched per frame. Still ours, lowest migration priority. |
| X | FFI / GPU / OS handoff buffer | Must be plain contiguous memory, so it goes VmReservation-backed. The one exception, X-os, is when the OS or driver OWNS the allocation; that sub-case is out of scope. |
| D | diagnostics / cold | Log strings, error reports, panics. |
| T | third-party internals reached through our code | Includes std internals. Recorded; we cannot move them ourselves. |
| C | compile-time / tooling only | Out of scope, with evidence that it never runs in the shipped binary. |
| U | unclassified | Must carry both candidate classes and what would decide between them. |

### ECS forms (closed; the FIRST form that fits is chosen)

| # | form | meaning |
|---|---|---|
| 1 | component | A per-entity datum on the entity it describes (archetype storage). |
| 2 | dense-component | Non-fragmenting: one contiguous buffer for all instances (docs/DENSE-COMPONENTS-PLAN.md). |
| 3 | enable-state | A boolean per entity, stored as EnableTag / EnableColumn (docs/DENSE-ENABLE-QUERY-PLAN.md). |
| 4 | relation | An edge between entities (crates/boyko_ecs/src/ecs/core/relationship). |
| 5 | event | A transient message that one system produces and others consume, carried by the kernel event buffers and the command channel. |
| 6 | resource-column | A singleton table that is not per entity, owned by a Resource on kernel columns. |
| 7 | system-scratch | Per-system transient data rebuilt on every run, held on a kernel ScratchColumn owned by the system (its Local, or KF-44 for an exclusive system). Rev 2 decides this backing; the frame arena of the allocator design space is not built. |
| 8 | scope-arena | Scheduler and pool transients, held in the kernel scope arena (block.rs). |
| 9 | kernel-internal | The ECS's own storage bookkeeping. It moves onto the memory library, which is the SDK itself. |
| 10 | new-kernel-feature:&lt;name> | Allowed only after every form above has been refuted with evidence. |
| 11 | out-of-scope:&lt;os-owned\|driver-owned\|compile-time\|test-only\|third-party> | Must be backed by evidence. `third-party` is only for a type that a third-party crate imposes (eframe / egui). std internals are `os-owned`, and since rev 3 only while they run before steady state: a std path conversion reachable after steady state is resource-column (the rev-3 path rule, section Decisions). `driver-owned` is narrowed to allocations the driver or OS owns itself (0 rows after gap 4). |
| 12 | diagnostics | NEW in rev 2 (gap 6), narrowed in rev 3. Cold diagnostic TEXT and ERROR payloads, built only on error / report paths. Home per site: a `&'static str` or structured code; a boyko_log ring record (formatting deferred to the drain); or a cold VmReservation-backed byte arena. In scope (our code allocates it), never kernel-internal. Rung 6. NOT this form (rev 3): numeric captures (profiler samples, GPU readback words, histograms, bit matrices), which are system-scratch when rebuilt per call and resource-column when retained across frames; and per-frame text, which is system-scratch by its growth. |

**Rev-2 vocabulary changes.**

- `diagnostics` is a form (gap 6). It replaces the two rev-1 spellings for D rows: `kernel-internal` ("the diagnostics substrate", 257 rows) and the non-vocabulary `out-of-scope:diagnostics` (61 ui-input rows). After rev 2 no D row is kernel-internal, including the two lanes (writer reconciliations W1 and W2). Three D rows keep another form by design: the two std panic payloads (`out-of-scope:os-owned`) and `boyko_rhi_vulkan/src/device.rs:688` (resource-column).
- `out-of-scope:std-internal` is retired. Its two rows (the ui hot-reload `std::fs::metadata` path conversion) are superseded by the ui-lane, which decided them resource-column (UL-D2, an in-house `GetFileAttributesExW` on a pre-encoded path).
- `out-of-scope:third-party` is admitted for types a third-party crate imposes (app-demo's 10 eframe / egui rows). The two codec-tools rows that carried it were std internals and are re-tagged `out-of-scope:os-owned` (writer change W6).
- `out-of-scope:driver-owned` has 0 rows (gap 4).
- Owning entities: `window` is a real entity type (gap 8, KF-46, engine Q3). `asset` is new (writer change W4): an asset entity under engine Q1 for the kinds the closed list does not name (font, sprite sheet). Mesh and material keep their names.

**Rev-3 vocabulary changes.**

- `diagnostics` is narrowed to text and error payloads (item 4). The numeric captures and the per-frame text that rev 2 put in it are re-formed by what their own notes prescribe: system-scratch for a capture rebuilt per call or per frame, resource-column for a capture retained across frames. Their class stays D, so they stay in rung 6.
- One path rule for std path conversions (item 6): reachable after steady state (hot-reload polls, log rotation and on-demand sink opens, asset load, save / load, end-of-run dumps) means resource-column, an in-house UTF-16 Win32 FFI call on a path encoded once into the owning record; boot-only means `out-of-scope:os-owned`.
- No supplementary rows (item 5). The 38 app-demo std-internal sites are counted rows, and `out-of-scope:third-party` is left on the 10 eframe / egui rows only.
- Owning entities: the three raw relation-endpoint spellings are normalised (M2). `asset` also names a datum that spans several asset kinds (the retire pass, the staging record, the `Assets<T>` pin bit).
- Gate exception (M5): a row whose container is already kernel storage (`other:ScratchColumn`, the two gap-2 physics rows) is a row for its form decision only. A migration gate that counts std heap must not count it.

### Owning entities

`body`, `soft-body`, `particle`, `mesh`, `material`, `asset`, `light`, `widget`, `text-run`, `window`, `observer`, `prefab`, `system`, `schedule`, `pool`, `relation-endpoint`, `none`. `relation-endpoint` merges the raw values that name a relation's parent or target entity. Kinds the closed list names but no row uses: collider, contact-pair, island, joint. `none` is legal only for resource-column, kernel-internal, diagnostics, scope-arena and out-of-scope.

**How to read a row.** One row = one site that owns or creates heap memory in non-test code: a field, static, local, return or param. The other fields are:

- **`class`** is the lifetime/ownership letter.
- **`ecs_form`** is the decision: the first form in the closed order that fits. Each row's `form_note` gives one sentence for every earlier form it skipped.
- **`owning_entity`** is the entity type the datum belongs to in the unified model. `none` is legal only for resource-column, kernel-internal, diagnostics, scope-arena and out-of-scope.
- **`memory destination`** is the *recount's* memory destination. It often names a `NEW:` primitive that the ECS-form pass later refuted (section (d)). Where the two disagree, `ecs_form` and `form_note` are the decision.
- **`plan_ref`** names the plan that already decides the row, if any.

**Not a row.** Any hit in doc comments, strings, `#[cfg(test)]` / `#[test]` code, trait bounds, type-alias declarations and `&[T]` / `&str` borrows. Every such hit is accounted for as a `non_row`, with a reason, in the group JSON ledger. The non_row counts are in section (a). The group ledgers (`<group>.ecsform.json`, with their recount and builder scripts) sit in the synthesis session's scratchpad and are not committed. The durable record is this index, the thirteen group files under `ledger/`, and the TSV; every field of every row is reproduced in its group file.

**Escaping.** In this document, `<` outside code spans is HTML-escaped as `&lt;`. Evidence cells are verbatim code spans.


**Section letters in row notes.** Row notes written before rev 2 name the rev-1 single-file sections: (a) totals, (b) entity model, (c) kernel features, (d) primitives refuted, (e) plan conflicts, (f) rows, (g) order of work, (h) what the ledger is not, (i) undecided forms. In rev 2 (a)-(e) and (g)-(h) are sections of this index under those names, (f) is the thirteen group files, and (i) is the section Decisions, which decides every item it listed.

**SUPERSEDED rows.** A row of ui-input whose site the ui-lane re-censused on its own tree is kept in `ledger/ui-input.md`, marked SUPERSEDED with a pointer to the lane row. It is not counted in the totals and not written to the TSV. All 122 boyko_ui rows of ui-input are superseded: the lane carries every one of them (evidence identical except one spelling, lines re-anchored) and adds two, so counting both would count 122 sites twice. The lane report's own "27 superseded" counts the rows whose content changed (cause lane 8, lane+decision 2, decision 17); the other 95 are superseded by identity.


**SUPERSEDED rows of render (rev 3).** The ui-lane rewrote the seven files of `crates/boyko_render/src/ui` (+2.7k lines) and now censuses them whole, so the 8 joltab rows there are SUPERSEDED by lane rows, and so is `bindless.rs:387`, whose function the lane changed (item 1). They stay in `ledger/render.md`, marked, not counted, not in the TSV. The 11 joltab non_rows in the same seven files are dropped from the non_row total for the same reason, as are the 123 boyko_ui non_rows of ui-input (M1).

## Totals

### Per group

| group | tree | rows | active | superseded | non_rows | supplementary | active by class |
|---|---|---|---|---|---|---|---|
| [ecs-storage](ledger/ecs-storage.md) | D:/wt/joltab | 109 | 109 | 0 | 498 | 0 | K 30, E 1, R 9, F 42, B 24, D 1, T 2 |
| [ecs-schedule](ledger/ecs-schedule.md) | D:/wt/joltab | 166 | 166 | 0 | 666 | 0 | K 35, R 9, F 6, S 6, B 96, D 14 |
| [ecs-services](ledger/ecs-services.md) | D:/wt/joltab | 77 | 77 | 0 | 549 | 0 | K 7, E 3, R 10, F 14, B 21, X 1, D 16, T 5 |
| [pool-utils-log](ledger/pool-utils-log.md) | D:/wt/joltab | 131 | 131 | 0 | 712 | 0 | K 3, R 3, S 5, B 27, D 14, T 23, C 56 |
| [physics-scene-math](ledger/physics-scene-math.md) | D:/wt/joltab | 95 | 95 | 0 | 562 | 0 | E 21, R 3, F 20, B 51 |
| [render](ledger/render.md) | D:/wt/joltab | 169 | 160 | 9 | 322 | 0 | K 1, R 6, F 5, B 66, D 80, C 2 |
| [rhi](ledger/rhi.md) | D:/wt/joltab | 64 | 64 | 0 | 289 | 0 | K 5, R 6, F 28, B 15, X 1, D 2, C 7 |
| [ui-input](ledger/ui-input.md) | D:/wt/joltab | 205 | 83 | 122 | 48 | 0 | R 8, B 31, D 44 |
| [app-demo](ledger/app-demo.md) | D:/wt/joltab | 256 | 256 | 0 | 84 | 0 | F 25, B 39, D 182, T 10 |
| [codec-tools](ledger/codec-tools.md) | D:/wt/joltab | 527 | 527 | 0 | 285 | 0 | B 50, X 3, D 1, T 2, C 471 |
| [macros-aether](ledger/macros-aether.md) | D:/wt/joltab | 462 | 462 | 0 | 116 | 0 | K 5, C 457 |
| [ui-lane](ledger/ui-lane.md) | D:/wt/ui | 196 | 196 | 0 | 203 | 0 | R 8, F 40, B 78, D 17, T 2, C 51 |
| [reflect-lane](ledger/reflect-lane.md) | D:/wt/reflect | 24 | 24 | 0 | 158 | 0 | F 2, D 5, C 17 |
| **total** |  | **2481** | **2350** | **131** | **4492** | **0** |  |

Sum of the groups' active rows = 2350 = the data-line count of `runtime-data-ledger.tsv` (2350).

### Active rows by crate x ecs_form

|  | component | dense-component | enable-state | relation | event | resource-column | system-scratch | scope-arena | kernel-internal | diagnostics | out-of-scope:compile-time | out-of-scope:os-owned | out-of-scope:test-only | out-of-scope:third-party | total |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| aether_lang | . | . | . | . | . | . | . | . | . | . | 263 | . | . | . | 263 |
| boyko_app | . | . | . | . | . | 51 | 22 | . | 4 | 124 | . | 19 | . | . | 220 |
| boyko_demo | . | 10 | . | . | . | 5 | 8 | . | 3 | . | . | . | . | 10 | 36 |
| boyko_diag | . | . | . | . | . | . | . | . | 1 | . | 7 | . | 20 | . | 28 |
| boyko_ecs | 12 | 2 | . | 26 | 11 | 10 | 54 | 6 | 210 | 22 | . | 1 | . | . | 354 |
| boyko_fontbake | . | . | . | . | . | 7 | 1 | . | . | . | 36 | . | . | . | 44 |
| boyko_image | . | . | . | . | . | . | 17 | . | . | 1 | . | . | . | . | 18 |
| boyko_input | . | . | . | . | 2 | 23 | 14 | . | . | 44 | . | . | . | . | 83 |
| boyko_log | . | . | . | . | . | 12 | . | . | 4 | 10 | 21 | 6 | 8 | . | 61 |
| boyko_macros | . | . | . | . | . | . | . | . | 5 | . | 211 | . | . | . | 216 |
| boyko_physics | . | 40 | . | . | . | 2 | 31 | . | . | . | . | . | . | . | 73 |
| boyko_reflect | . | . | . | . | . | . | . | . | . | 5 | . | . | . | . | 5 |
| boyko_render | 6 | 6 | 8 | . | . | 17 | 63 | . | 4 | 75 | . | . | 5 | . | 184 |
| boyko_rhi | . | . | . | . | . | 2 | . | . | . | . | . | . | . | . | 2 |
| boyko_rhi_vulkan | 4 | 2 | . | . | . | 7 | 41 | . | . | 1 | . | . | 7 | . | 62 |
| boyko_scene | . | 1 | . | 1 | 1 | 4 | 4 | . | . | . | . | . | . | . | 11 |
| boyko_sdf_math | . | . | . | . | . | . | 11 | . | . | . | . | . | . | . | 11 |
| boyko_serialize | . | . | . | . | . | 2 | 28 | . | . | . | . | . | . | . | 30 |
| boyko_shaderdsl | . | . | . | . | . | . | . | . | . | . | 468 | . | . | . | 468 |
| boyko_threadpool | . | . | . | . | . | . | . | 4 | 26 | 2 | . | 4 | . | . | 36 |
| boyko_ui | 10 | . | . | 4 | 2 | 10 | 76 | . | 4 | 17 | . | 1 | . | . | 124 |
| boyko_utils | . | . | . | . | . | 3 | . | . | 3 | . | . | . | . | . | 6 |
| prof_decode | . | . | . | . | . | . | . | . | . | . | 15 | . | . | . | 15 |
| **total** | **32** | **61** | **8** | **31** | **16** | **155** | **370** | **10** | **264** | **301** | **1021** | **31** | **40** | **10** | **2350** |

### Active rows by crate x class

|  | K | E | R | F | S | B | X | D | T | C | total |
|---|---|---|---|---|---|---|---|---|---|---|---|
| aether_lang | . | . | . | . | . | . | . | . | . | 263 | 263 |
| boyko_app | . | . | . | 7 | . | 31 | . | 182 | . | . | 220 |
| boyko_demo | . | . | . | 18 | . | 8 | . | . | 10 | . | 36 |
| boyko_diag | . | . | . | . | . | . | . | . | 1 | 27 | 28 |
| boyko_ecs | 72 | 4 | 28 | 64 | 6 | 141 | 1 | 31 | 7 | . | 354 |
| boyko_fontbake | . | . | . | . | . | 8 | . | . | . | 36 | 44 |
| boyko_image | . | . | . | . | . | 17 | . | 1 | . | . | 18 |
| boyko_input | . | . | 8 | . | . | 31 | . | 44 | . | . | 83 |
| boyko_log | . | . | . | . | . | 7 | . | 10 | 15 | 29 | 61 |
| boyko_macros | 5 | . | . | . | . | . | . | . | . | 211 | 216 |
| boyko_physics | . | 21 | . | 16 | . | 36 | . | . | . | . | 73 |
| boyko_reflect | . | . | . | . | . | . | . | 5 | . | . | 5 |
| boyko_render | 1 | . | 6 | 18 | . | 74 | . | 80 | . | 5 | 184 |
| boyko_rhi | 2 | . | . | . | . | . | . | . | . | . | 2 |
| boyko_rhi_vulkan | 3 | . | 6 | 28 | . | 15 | 1 | 2 | . | 7 | 62 |
| boyko_scene | . | . | 3 | 4 | . | 4 | . | . | . | . | 11 |
| boyko_sdf_math | . | . | . | . | . | 11 | . | . | . | . | 11 |
| boyko_serialize | . | . | . | . | . | 25 | 3 | . | 2 | . | 30 |
| boyko_shaderdsl | . | . | . | . | . | . | . | . | . | 468 | 468 |
| boyko_threadpool | . | . | . | . | 5 | 20 | . | 4 | 7 | . | 36 |
| boyko_ui | . | . | 8 | 27 | . | 70 | . | 17 | 2 | . | 124 |
| boyko_utils | 3 | . | 3 | . | . | . | . | . | . | . | 6 |
| prof_decode | . | . | . | . | . | . | . | . | . | 15 | 15 |
| **total** | **86** | **25** | **62** | **182** | **11** | **498** | **5** | **376** | **44** | **1061** | **2350** |

### Active rows by class x ecs_form

|  | component | dense-component | enable-state | relation | event | resource-column | system-scratch | scope-arena | kernel-internal | diagnostics | out-of-scope:compile-time | out-of-scope:os-owned | out-of-scope:test-only | out-of-scope:third-party | total |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| K | 2 | 1 | . | 5 | . | 5 | . | . | 73 | . | . | . | . | . | 86 |
| E | . | 19 | . | 4 | . | 2 | . | . | . | . | . | . | . | . | 25 |
| R | 9 | 5 | . | 2 | 9 | 26 | . | . | 11 | . | . | . | . | . | 62 |
| F | 4 | 11 | 4 | . | 2 | 1 | 129 | . | 31 | . | . | . | . | . | 182 |
| S | . | . | . | . | . | . | . | 10 | 1 | . | . | . | . | . | 11 |
| B | 16 | 25 | 4 | 20 | 5 | 54 | 216 | . | 133 | . | . | 25 | . | . | 498 |
| X | 1 | . | . | . | . | . | 3 | . | 1 | . | . | . | . | . | 5 |
| D | . | . | . | . | . | 51 | 22 | . | . | 301 | . | 2 | . | . | 376 |
| T | . | . | . | . | . | 16 | . | . | 14 | . | . | 4 | . | 10 | 44 |
| C | . | . | . | . | . | . | . | . | . | . | 1021 | . | 40 | . | 1061 |
| **total** | **32** | **61** | **8** | **31** | **16** | **155** | **370** | **10** | **264** | **301** | **1021** | **31** | **40** | **10** | **2350** |

### Active rows by container

| container | rows |
|---|---|
| Vec | 1088 |
| String | 890 |
| Box&lt;[T]> | 61 |
| Box&lt;T> | 53 |
| other:OsString/String (std::env) | 28 |
| other:PathBuf | 24 |
| Arc | 22 |
| HashMap | 20 |
| Box&lt;dyn> | 19 |
| other:fixedbitset::FixedBitSet | 17 |
| other:thread_local! os-key Value&lt;T> (std AlignedSystemBox, System.alloc) | 14 |
| other:std::fs internals (Vec&lt;u16> wide path) | 12 |
| other:SmallList4 (4 inline, Vec spill - component/enable/enable_store.rs:765-770) | 10 |
| other:Vec&lt;u16> (std::fs path conversion) | 10 |
| Cow | 8 |
| PathBuf | 6 |
| other:SparseMap (boyko_utils; 3 x Vec inside) | 5 |
| other:OsString / ReadDir (std) | 5 |
| other:std stable-sort scratch (Vec&lt;T> inside alloc::slice::stable_sort) | 5 |
| other:proc_macro2::Ident | 5 |
| HashSet | 4 |
| other:ScopeBlock chunk (std::alloc::alloc) | 3 |
| other:LiveBitmap (Vec&lt;u64> inside, component/dense/live_bitmap.rs:31) | 3 |
| other:OsString | 3 |
| other:std Windows UTF-16 path buffer | 3 |
| other:crossbeam_queue::ArrayQueue | 2 |
| other:std::thread internals | 2 |
| other:BufWriter (8 KiB Vec&lt;u8> buffer) | 2 |
| other:ScratchColumn (kernel column on VmReservation; engine storage, not std heap) | 2 |
| other:UiParseReport (Vec&lt;(usize,u16,String)> x2) | 2 |
| other:std-internal Vec&lt;u16> (Windows path conversion) | 2 |
| other:std::alloc::alloc_zeroed | 1 |
| other:SparseMap&lt;Vec> (boyko_utils SparseMap: 3 x Vec, plus one inner Vec per active pattern) | 1 |
| other:VisitedSet (Vec&lt;u64> inside; iters/query/relation/traverse_iter.rs:51) | 1 |
| other:RawBlob (hand-rolled std::alloc alloc/realloc/dealloc byte arena) | 1 |
| VecDeque | 1 |
| other:std::alloc::alloc raw block | 1 |
| other:std-internal Vec&lt;u16> (Windows path -> UTF-16 inside File::open) | 1 |
| other:std::alloc raw chunk | 1 |
| other:std::alloc raw cell | 1 |
| other:crossbeam_deque::Injector | 1 |
| other:crossbeam_deque::Worker buffer | 1 |
| other:std::process::Command + Output{stdout: Vec&lt;u8>} | 1 |
| other:crossbeam-epoch per-thread Local (+ std TLS destructor list) | 1 |
| other:OnceLock&lt;Mutex&lt;InternerState>> | 1 |
| other:SparseSlotMap | 1 |
| other:#[global_allocator] shim (CountingAlloc forwarding to std::alloc::System) | 1 |
| other:threadpool install scope | 1 |
| other:threadpool spawned task cell | 1 |
| other:std stable-sort scratch (driftsort BufT) | 1 |
| BTreeSet | 1 |

### Active rows by kind, growth, addr_cached

- **kind:** local 1557, field 442, return 239, param 72, static 40
- **growth:** once 1956, highwater 173, unknown 125, perframe 96
- **addr_cached:** no 2271, yes 79

**Double representation, by design.** 23 rows are *use sites* of one of our own heap-holding types (SmallList4, SparseMap, LiveBitmap, VisitedSet, SparseSlotMap, UiParseReport). Those types' own `Vec` fields are rows too, in the group that owns the type. Migrating the type retires both; count that storage once.

## The entity model

Every `owning_entity` the active rows imply, with the components, dense components, relations, enable-states and events each one carries in the unified model.

|  | component | dense-component | enable-state | relation | event | resource-column | system-scratch | scope-arena | kernel-internal | diagnostics | out-of-scope:compile-time | out-of-scope:os-owned | out-of-scope:test-only | out-of-scope:third-party | total |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| body | . | 12 | . | . | . | . | . | . | . | . | . | . | . | . | 12 |
| soft-body | . | 36 | . | . | . | . | . | . | . | . | . | . | . | . | 36 |
| particle | . | 2 | . | . | . | . | . | . | . | . | . | . | . | . | 2 |
| mesh | . | 3 | . | 1 | . | . | 18 | . | . | . | . | . | . | . | 22 |
| material | . | 2 | . | . | . | 1 | 11 | . | . | . | . | . | . | . | 14 |
| asset | 4 | 2 | . | . | . | . | . | . | . | . | . | . | . | . | 6 |
| light | . | . | 8 | . | . | 2 | . | . | . | . | . | . | . | . | 10 |
| widget | 14 | . | . | 4 | 2 | . | . | . | . | . | . | . | . | . | 20 |
| text-run | . | . | . | . | . | . | 3 | . | . | . | . | . | . | . | 3 |
| window | 4 | 2 | . | . | 2 | . | 5 | . | . | . | . | . | . | . | 13 |
| observer | . | . | . | 5 | . | . | . | . | . | . | . | . | . | . | 5 |
| prefab | 4 | . | . | . | . | . | . | . | . | . | . | . | . | . | 4 |
| system | 6 | 2 | . | 18 | 8 | 8 | 313 | . | 19 | . | . | . | . | . | 374 |
| schedule | . | . | . | . | 4 | . | 20 | . | 78 | . | . | . | . | . | 102 |
| pool | . | . | . | . | . | . | . | 10 | 30 | 2 | . | . | . | . | 42 |
| relation-endpoint | . | . | . | 3 | . | . | . | . | . | . | . | . | . | . | 3 |
| none | . | . | . | . | . | 144 | . | . | 137 | 299 | 1021 | 31 | 40 | 10 | 1682 |
| **total** | **32** | **61** | **8** | **31** | **16** | **155** | **370** | **10** | **264** | **301** | **1021** | **31** | **40** | **10** | **2350** |

Mappings forced by the closed list, each recorded on its rows: texture -> `material`; system set -> `system`; scene node -> `system` (the consumer of the ChildOf detach event); boid -> `particle`. Entity types this ledger introduces: **`observer`**, **`prefab`** (rev 1) and **`asset`** (rev 2). **`window`** is a real entity type in rev 2.

### body (12 rows)

**Rigid body** (and the demo's 2D balls). A body IS an entity. Rev 2 follows the physics design and its decisions:
- **identity** is a stable slot of the `PhysicsBody` dense group, not the archetype row (D1). The slot never moves, so no per-body datum can be renamed by a despawn (latent defect A, section Defects);
- **dense-component** `BodyGate {flags, asleep, below}`: the sleep latch and its debounce counter (rows `boyko_physics/src/resources.rs:3057/3062` and their constructors `:3104/3105`, writer change W3). main:docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md:304 "| `BodyGate` {flags, asleep, below} | group column, untracked | body |". The rev-1 enable-state `Sleeping` and component `SleepCounter` are withdrawn, and with them KF-16 and KF-18;
- **resource-column** `PairCache` (warm impulses + SAT axis), keyed by body slots and double-buffered, with the D15 `fresh_step` skip (gap 2; rows `solver/warm_start.rs:214`, `narrowphase/axis_cache.rs:128`). main:docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md:307 "| `PairCache` (warm impulses + axis) | persistent resource columns, double-buffered | — | D5/D15 |";
- **events**: contact and sensor enter/exit, sleep/wake transitions (physics Q1, Q3). No `Touching` relation and no `Contact` component;
- **dense-component** Position / Velocity / Radius for the demo balls (rows `boyko_demo/src/sim/resources.rs:191-214`), KF-19 and KF-20 (= physics K3 and K4).

Not entities: contact pairs, manifolds, islands, colours and grid cells (main:docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md:291 "- No pair, contact or island entities.").

| form | row | owner | container&lt;elem&gt; | class | group |
|---|---|---|---|---|---|
| dense-component | crates/boyko_demo/src/sim/resources.rs:191 `pub pos: Vec<Position>,` | BallSnapshot::pos | `Vec<Position>` | F | [app-demo](ledger/app-demo.md) |
| dense-component | crates/boyko_demo/src/sim/resources.rs:193 `pub vel: Vec<Velocity>,` | BallSnapshot::vel | `Vec<Velocity>` | F | [app-demo](ledger/app-demo.md) |
| dense-component | crates/boyko_demo/src/sim/resources.rs:195 `pub radius: Vec<f32>,` | BallSnapshot::radius | `Vec<f32>` | F | [app-demo](ledger/app-demo.md) |
| dense-component | crates/boyko_demo/src/sim/resources.rs:199 `pub touched: Vec<bool>,` | BallSnapshot::touched | `Vec<bool>` | F | [app-demo](ledger/app-demo.md) |
| dense-component | crates/boyko_demo/src/sim/resources.rs:211 `pos: Vec::with_capacity(max_balls),` | BallSnapshot (with_capacity initializer) | `Vec<(see field)>` | F | [app-demo](ledger/app-demo.md) |
| dense-component | crates/boyko_demo/src/sim/resources.rs:212 `vel: Vec::with_capacity(max_balls),` | BallSnapshot (with_capacity initializer) | `Vec<(see field)>` | F | [app-demo](ledger/app-demo.md) |
| dense-component | crates/boyko_demo/src/sim/resources.rs:213 `radius: Vec::with_capacity(max_balls),` | BallSnapshot (with_capacity initializer) | `Vec<(see field)>` | F | [app-demo](ledger/app-demo.md) |
| dense-component | crates/boyko_demo/src/sim/resources.rs:214 `touched: Vec::with_capacity(max_balls),` | BallSnapshot (with_capacity initializer) | `Vec<(see field)>` | F | [app-demo](ledger/app-demo.md) |
| dense-component | crates/boyko_physics/src/resources.rs:3057 `asleep: Vec<bool>,` | IslandSleep::asleep | `Vec<bool>` | E | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/resources.rs:3062 `below_count: Vec<u16>,` | IslandSleep::below_count | `Vec<u16>` | E | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/resources.rs:3104 `asleep: Vec::with_capacity(rows),` | IslandSleep::with_capacity (-> IslandSleep::asleep) | `Vec<bool>` | B | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/resources.rs:3105 `below_count: Vec::with_capacity(rows),` | IslandSleep::with_capacity (-> IslandSleep::below_count) | `Vec<u16>` | B | [physics-scene-math](ledger/physics-scene-math.md) |

### soft-body (36 rows)

**Soft body.** It is an entity; its particles are not. Rev 2 (gap 3, physics Q2): the particle state AND the topology are per-body segments of the K7 **segmented dense column** (dense-component, 36 rows of `boyko_physics/src/soft/component.rs`), and the per-substep `prev_*` / `coupling_*` / `sc_*` fields are one **shared system-scratch** column sized to the largest body, because bodies are stepped serially. main:docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md:313 "| `SoftBody` particle columns + topology | kernel segmented dense column (K7), recommended | soft body | Q2 |" ; main:docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md:314 "| `SoftBody` per-substep scratch (10) | shared system scratch | — | Bodies are stepped serially (`soft/colored.rs:699`) |". Particles as entities are rejected: dense slots are reused last-in-first-out, so after despawns a new body's particles would scatter and break the solver's one raw base per attribute.

| form | row | owner | container&lt;elem&gt; | class | group |
|---|---|---|---|---|---|
| dense-component | crates/boyko_physics/src/soft/component.rs:71 `pub pos_x: Vec<f32>,` | SoftBody::pos_x | `Vec<f32>` | E | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:73 `pub pos_y: Vec<f32>,` | SoftBody::pos_y | `Vec<f32>` | E | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:75 `pub pos_z: Vec<f32>,` | SoftBody::pos_z | `Vec<f32>` | E | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:83 `pub vel_x: Vec<f32>,` | SoftBody::vel_x | `Vec<f32>` | E | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:85 `pub vel_y: Vec<f32>,` | SoftBody::vel_y | `Vec<f32>` | E | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:87 `pub vel_z: Vec<f32>,` | SoftBody::vel_z | `Vec<f32>` | E | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:89 `pub inv_mass: Vec<f32>,` | SoftBody::inv_mass | `Vec<f32>` | E | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:91 `pub c_a: Vec<u32>,` | SoftBody::c_a | `Vec<u32>` | E | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:93 `pub c_b: Vec<u32>,` | SoftBody::c_b | `Vec<u32>` | E | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:95 `pub c_rest: Vec<f32>,` | SoftBody::c_rest | `Vec<f32>` | E | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:98 `pub c_compliance: Vec<f32>,` | SoftBody::c_compliance | `Vec<f32>` | E | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:105 `pub t0: Vec<u32>,` | SoftBody::t0 | `Vec<u32>` | E | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:107 `pub t1: Vec<u32>,` | SoftBody::t1 | `Vec<u32>` | E | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:109 `pub t2: Vec<u32>,` | SoftBody::t2 | `Vec<u32>` | E | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:111 `pub t3: Vec<u32>,` | SoftBody::t3 | `Vec<u32>` | E | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:115 `pub t_rest: Vec<f32>,` | SoftBody::t_rest | `Vec<f32>` | E | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:118 `pub t_compliance: Vec<f32>,` | SoftBody::t_compliance | `Vec<f32>` | E | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:327 `let mut t0 = Vec::with_capacity(k);` | SoftBody::from_tet_mesh (-> SoftBody::t0) | `Vec<u32>` | B | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:328 `let mut t1 = Vec::with_capacity(k);` | SoftBody::from_tet_mesh (-> SoftBody::t1) | `Vec<u32>` | B | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:329 `let mut t2 = Vec::with_capacity(k);` | SoftBody::from_tet_mesh (-> SoftBody::t2) | `Vec<u32>` | B | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:330 `let mut t3 = Vec::with_capacity(k);` | SoftBody::from_tet_mesh (-> SoftBody::t3) | `Vec<u32>` | B | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:331 `let mut t_rest = Vec::with_capacity(k);` | SoftBody::from_tet_mesh (-> SoftBody::t_rest) | `Vec<f32>` | B | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:374 `body.t_compliance = vec![tet_compliance; k];` | SoftBody::from_tet_mesh (-> SoftBody::t_compliance) | `Vec<f32>` | B | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:499 `let mut pos_x = Vec::with_capacity(n);` | SoftBody::build (-> SoftBody::pos_x) | `Vec<f32>` | B | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:500 `let mut pos_y = Vec::with_capacity(n);` | SoftBody::build (-> SoftBody::pos_y) | `Vec<f32>` | B | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:501 `let mut pos_z = Vec::with_capacity(n);` | SoftBody::build (-> SoftBody::pos_z) | `Vec<f32>` | B | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:511 `let vel_x = vec![0.0; n];` | SoftBody::build (-> SoftBody::vel_x) | `Vec<f32>` | B | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:512 `let vel_y = vec![0.0; n];` | SoftBody::build (-> SoftBody::vel_y) | `Vec<f32>` | B | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:513 `let vel_z = vec![0.0; n];` | SoftBody::build (-> SoftBody::vel_z) | `Vec<f32>` | B | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:514 `let inv_mass = inv_masses.to_vec();` | SoftBody::build (-> SoftBody::inv_mass) | `Vec<f32>` | B | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:516 `let mut c_a = Vec::with_capacity(m);` | SoftBody::build (-> SoftBody::c_a) | `Vec<u32>` | B | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:517 `let mut c_b = Vec::with_capacity(m);` | SoftBody::build (-> SoftBody::c_b) | `Vec<u32>` | B | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:524 `Some(r) => r.to_vec(),` | SoftBody::build (-> SoftBody::c_rest) | `Vec<f32>` | B | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:526 `let mut out = Vec::with_capacity(m);` | SoftBody::build (-> SoftBody::c_rest) | `Vec<f32>` | B | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:540 `Compliance::Uniform(alpha) => vec![alpha; m],` | SoftBody::build (-> SoftBody::c_compliance) | `Vec<f32>` | B | [physics-scene-math](ledger/physics-scene-math.md) |
| dense-component | crates/boyko_physics/src/soft/component.rs:541 `Compliance::PerEdge(slice) => slice.to_vec(),` | SoftBody::build (-> SoftBody::c_compliance) | `Vec<f32>` | B | [physics-scene-math](ledger/physics-scene-math.md) |

### particle (2 rows)

**Particle (demo boid).** Dense-component `BoidPrev`: the boid's pre-step (pos, vel), read by its neighbours (rows `boyko_demo/src/sim/resources.rs:134/142`). The closed list has no "agent" kind. Needs KF-19.

| form | row | owner | container&lt;elem&gt; | class | group |
|---|---|---|---|---|---|
| dense-component | crates/boyko_demo/src/sim/resources.rs:134 `pub state: Vec<BoidState>,` | BoidSnapshot::state | `Vec<BoidState>` | F | [app-demo](ledger/app-demo.md) |
| dense-component | crates/boyko_demo/src/sim/resources.rs:142 `state: Vec::with_capacity(max_boids),` | BoidSnapshot::state (with_capacity initializer) | `Vec<BoidState>` | F | [app-demo](ledger/app-demo.md) |

### mesh (22 rows)

**Mesh** (an asset). Under engine decision Q1 an asset IS an entity (main:docs/unification/ENGINE-RUNTIME-ECS-DECISIONS.md:23 "| Q1 | Assets as entities | **(a) Assets are entities** | yes |"). Rev 3 applies it to every asset row (item 2):
- **dense-component**: the GPU value is a column of the mesh K3 group with `RELEASE = Deferred` (main:docs/unification/ENGINE-RUNTIME-ECS-DESIGN.md:216 "- **Value and release.** The asset value is a column of a **K3 dense group with `RELEASE = Deferred`** on the asset"). The handle lists collected only to walk the store (`gpu_upload.rs:216`, `mesh_assets.rs:582`) disappear into an in-place walk of the group, and a fill-rejected or retired value waits in the group's `dying` list for the horizon release K6' (`mesh_assets.rs:712`; KF-49);
- **relation**: the refcount is a count-only relation, instance -> asset (`asset_refs.rs:99`; KF-48, engine EK15b: main:docs/unification/ENGINE-RUNTIME-ECS-DECISIONS.md:35 "19 copies of 7 data become 7. Refcounting becomes a count-only relation (EK15b), and asset change");
- **system-scratch**: the decode payloads (`mesh_data.rs:28/30` and the glb / obj loader buffers) exist only from decode to upload. They sit on the load system's ScratchColumn lanes (KF-09), and the `Staged<A::Cpu>` component on the asset entity holds a Copy span into them;
- the SDF bake grids and the TLAS instance list stay **system-scratch**, and the mesh index byte images are system-scratch (gap 4).

| form | row | owner | container&lt;elem&gt; | class | group |
|---|---|---|---|---|---|
| dense-component | crates/boyko_render/src/gpu_upload.rs:216 `let mut pending: Vec<Handle<MeshGpu>> = Vec::with_capacity(assets.len());` | backfill_vb_geometry_slots::pending | `Vec<Handle<MeshGpu>>` | B | [render](ledger/render.md) |
| dense-component | crates/boyko_render/src/mesh_assets.rs:582 `let handles: Vec<Handle<MeshGpu>> = self.iter().map(\|(h, _)\| h).collect();` | MeshAssetsExt::destroy::handles | `Vec<Handle<MeshGpu>>` | B | [render](ledger/render.md) |
| dense-component | crates/boyko_render/src/mesh_assets.rs:712 `orphans: Vec<(MeshGpu, u64)>,` | OrphanedMeshGpu::orphans | `Vec<(MeshGpu, u64)>` | R | [render](ledger/render.md) |
| relation | crates/boyko_scene/src/asset_refs.rs:99 `deltas: Vec<RefDelta>,` | RefcountDeltas::deltas | `Vec<RefDelta>` | R | [physics-scene-math](ledger/physics-scene-math.md) |
| system-scratch | crates/boyko_render/src/loaders/glb.rs:786 `) -> Result<(Vec<Vertex>, Vec<u32>), AssetError> {` | decode_primitive -> Result&lt;(Vec&lt;Vertex>, Vec&lt;u32>), AssetError> | `Vec<Vertex and u32 (a pair of Vecs)>` | B | [render](ledger/render.md) |
| system-scratch | crates/boyko_render/src/loaders/glb.rs:831 `let mut vertices = Vec::with_capacity(pos.count);` | decode_primitive::vertices | `Vec<Vertex>` | B | [render](ledger/render.md) |
| system-scratch | crates/boyko_render/src/loaders/glb.rs:857 `let mut indices = Vec::with_capacity(idx.count);` | decode_primitive::indices | `Vec<u32>` | B | [render](ledger/render.md) |
| system-scratch | crates/boyko_render/src/loaders/glb.rs:915 `let mut vertices: Vec<Vertex> = Vec::new();` | GlbMeshLoader::decode::vertices (-> MeshData::vertices) | `Vec<Vertex>` | B | [render](ledger/render.md) |
| system-scratch | crates/boyko_render/src/loaders/glb.rs:916 `let mut indices: Vec<u32> = Vec::new();` | GlbMeshLoader::decode::indices (-> MeshData::indices) | `Vec<u32>` | B | [render](ledger/render.md) |
| system-scratch | crates/boyko_render/src/loaders/obj.rs:183 `fn dedup_corners(corners: Vec<CornerRecord>) -> (Vec<Vertex>, Vec<u32>) {` | dedup_corners(corners: Vec&lt;CornerRecord>) -> (Vec&lt;Vertex>, Vec&lt;u32>) | `Vec<Vertex and u32 (a pair of Vecs); param Vec<CornerRecord>>` | B | [render](ledger/render.md) |
| system-scratch | crates/boyko_render/src/loaders/obj.rs:187 `let mut vertices: Vec<Vertex> = Vec::with_capacity(corners.len());` | dedup_corners::vertices (-> MeshData::vertices) | `Vec<Vertex>` | B | [render](ledger/render.md) |
| system-scratch | crates/boyko_render/src/loaders/obj.rs:188 `let mut indices: Vec<u32> = vec![0; corners.len()];` | dedup_corners::indices (-> MeshData::indices) | `Vec<u32>` | B | [render](ledger/render.md) |
| system-scratch | crates/boyko_render/src/mesh_assets.rs:339 `let index_bytes: Vec<u8> = match index_type {` | build_mesh_gpu::index_bytes | `Vec<u8>` | B | [render](ledger/render.md) |
| system-scratch | crates/boyko_render/src/mesh_assets.rs:341 `let mut bytes = Vec::with_capacity(indices.len() * 2);` | build_mesh_gpu::index_bytes (Uint16 arm) | `Vec<u8>` | B | [render](ledger/render.md) |
| system-scratch | crates/boyko_render/src/mesh_assets.rs:373 `let mut bytes = Vec::with_capacity(indices.len() * 4);` | build_mesh_gpu::index_bytes (Uint32 arm) | `Vec<u8>` | B | [render](ledger/render.md) |
| system-scratch | crates/boyko_render/src/mesh_data.rs:28 `pub vertices: Vec<Vertex>,` | MeshData::vertices | `Vec<Vertex>` | B | [render](ledger/render.md) |
| system-scratch | crates/boyko_render/src/mesh_data.rs:30 `pub indices: Vec<u32>,` | MeshData::indices | `Vec<u32>` | B | [render](ledger/render.md) |
| system-scratch | crates/boyko_rhi_vulkan/src/accel_build.rs:464 `let instances: Vec<_> = blas_addresses` | accel_build::build_tlas | `Vec<VkAccelerationStructureInstanceKHR (64 B, #[repr(C)])>` | B | [rhi](ledger/rhi.md) |
| system-scratch | crates/boyko_rhi_vulkan/src/mesh_sdf_texture.rs:187 `let grid = bake_dense_grid(mesh, &self.field);` | MeshSdfTexture::bake_and_upload | `Vec<i8 (snorm SDF code per voxel, grid_dim product)>` | B | [rhi](ledger/rhi.md) |
| system-scratch | crates/boyko_sdf_math/src/mesh_sdf.rs:791 `pub fn bake_dense_grid(mesh: &BakeMesh, field: &MeshSdfField) -> Vec<i8> {` | bake_dense_grid | `Vec<i8>` | B | [physics-scene-math](ledger/physics-scene-math.md) |
| system-scratch | crates/boyko_sdf_math/src/mesh_sdf.rs:800 `pub fn bake_dense_grid_with_bvh(mesh: &BakeMesh, bvh: &TriBvh, field: &MeshSdfField) -> Vec<i8> {` | bake_dense_grid_with_bvh | `Vec<i8>` | B | [physics-scene-math](ledger/physics-scene-math.md) |
| system-scratch | crates/boyko_sdf_math/src/mesh_sdf.rs:814 `let mut out = Vec::with_capacity(w * h * d);` | bake_dense_grid_with_bvh (out) | `Vec<i8>` | B | [physics-scene-math](ledger/physics-scene-math.md) |

### material (14 rows)

**Material** (textures included: the closed list has no texture kind). An asset entity under engine Q1. The GPU value (`TextureGpu`) is a K3 Deferred group column (**dense-component**): the teardown handle list `texture.rs:709` becomes an in-place walk and the orphan queue `texture.rs:928` becomes the group's `dying` list (KF-49). The decode payloads (`TextureData::rgba8`, the PNG decoder buffers, the font atlas texels) are **system-scratch** of the load system, from decode to upload (item 2).

| form | row | owner | container&lt;elem&gt; | class | group |
|---|---|---|---|---|---|
| dense-component | crates/boyko_render/src/texture.rs:709 `let handles: Vec<Handle<TextureGpu>> = self.iter().map(\|(h, _)\| h).collect();` | TextureAssetsExt::destroy::handles | `Vec<Handle<TextureGpu>>` | B | [render](ledger/render.md) |
| dense-component | crates/boyko_render/src/texture.rs:928 `orphans: Vec<(TextureGpu, u64)>,` | OrphanedTextureGpu::orphans | `Vec<(TextureGpu, u64)>` | R | [render](ledger/render.md) |
| resource-column | crates/boyko_fontbake/src/atlas.rs:629 `let pixels = r.take(px_len)?.to_vec();` | boyko_fontbake::atlas::read_bfont (let pixels) | `Vec<u8 (RGBA8 atlas texels)>` | B | [codec-tools](ledger/codec-tools.md) |
| system-scratch | crates/boyko_fontbake/src/atlas.rs:116 `pub pixels: Vec<u8>,` | boyko_fontbake::atlas::AtlasImage::pixels | `Vec<u8 (RGBA8 atlas texels)>` | B | [codec-tools](ledger/codec-tools.md) |
| system-scratch | crates/boyko_image/src/png.rs:60 `pub pixels: Vec<u8>,` | boyko_image::DecodedImage::pixels | `Vec<u8>` | B | [codec-tools](ledger/codec-tools.md) |
| system-scratch | crates/boyko_image/src/png.rs:378 `) -> Result<Vec<u8>, PngError> {` | boyko_image::png::unfilter | `Vec<u8 (unfiltered scanline bytes)>` | B | [codec-tools](ledger/codec-tools.md) |
| system-scratch | crates/boyko_image/src/png.rs:381 `let mut out = vec![0u8; row_bytes * height];` | boyko_image::png::unfilter (let out) | `Vec<u8>` | B | [codec-tools](ledger/codec-tools.md) |
| system-scratch | crates/boyko_image/src/png.rs:421 `fn expand_to_rgba(unfiltered: Vec<u8>, width: u32, height: u32, color_type: u8, bytes_per_sample: usize) -> Vec<u8> {` | boyko_image::png::expand_to_rgba | `Vec<u8 (RGBA pixels)>` | B | [codec-tools](ledger/codec-tools.md) |
| system-scratch | crates/boyko_image/src/png.rs:430 `let mut out = vec![0u8; px_count * dst_stride];` | boyko_image::png::expand_to_rgba (let out, color type 2) | `Vec<u8 (RGBA pixels)>` | B | [codec-tools](ledger/codec-tools.md) |
| system-scratch | crates/boyko_image/src/png.rs:441 `let mut out = vec![0u8; px_count * dst_stride];` | boyko_image::png::expand_to_rgba (let out, color type 0) | `Vec<u8 (RGBA pixels)>` | B | [codec-tools](ledger/codec-tools.md) |
| system-scratch | crates/boyko_image/src/png.rs:456 `let mut out = vec![0u8; px_count * dst_stride];` | boyko_image::png::expand_to_rgba (let out, color type 4) | `Vec<u8 (RGBA pixels)>` | B | [codec-tools](ledger/codec-tools.md) |
| system-scratch | crates/boyko_render/src/loaders/png_texture.rs:42 `fn narrow_to_rgba8(image: DecodedImage) -> Vec<u8> {` | narrow_to_rgba8 -> Vec&lt;u8> (-> TextureData::rgba8) | `Vec<u8>` | B | [render](ledger/render.md) |
| system-scratch | crates/boyko_render/src/loaders/png_texture.rs:50 `let mut out = Vec::with_capacity(image.pixels.len() / 2);` | narrow_to_rgba8::out (16-bit arm) | `Vec<u8>` | B | [render](ledger/render.md) |
| system-scratch | crates/boyko_render/src/texture_data.rs:28 `pub rgba8: Vec<u8>,` | TextureData::rgba8 | `Vec<u8>` | B | [render](ledger/render.md) |

### asset (6 rows)

**Asset** (NEW in rev 2, writer change W4; widened in rev 3): an asset entity of a kind the closed list does not name, or a datum that spans several asset kinds. Fonts and sprite sheets are asset entities (main:docs/unification/ENGINE-RUNTIME-ECS-DESIGN.md:513 "| mesh, material, texture, font, sprite-sheet, animation-clip, UI document | **yes, asset entities (Q1a)** |"), and `FontTable::fonts` / `UiSheetTable::sheets` are components on them; the glyph tables stay write-once CSR bytes on resource-owned columns. Rev 3 (item 2) adds the kind-generic rows of the asset kernel: the `Pinned` marker (`assets.rs:206`, component), the `Staged<A::Cpu>` staging record (`staging.rs:58`, component: main:docs/unification/ENGINE-RUNTIME-ECS-DESIGN.md:647 "| `AssetStaging<A>` | NonSend Vec | component `Staged<A::Cpu>` on the asset entity | AS5 |"), and the retire pass over the `dying` lists (`asset_refcount.rs:556`, `asset_refs.rs:149`, dense-component: main:docs/unification/ENGINE-RUNTIME-ECS-DESIGN.md:649 "| `DeferredFree` **(rev 2)** | Resource Vec | deleted: K3 `dying` + K6′ horizon (ED16) | AS2 |").

| form | row | owner | container&lt;elem&gt; | class | group |
|---|---|---|---|---|---|
| component | crates/boyko_ecs/src/ecs/core/asset/assets.rs:206 `pinned: LiveBitmap,` | Assets&lt;T>::pinned | `other:LiveBitmap (Vec<u64> inside, component/dense/live_bitmap.rs:31)<u64 bitmap word>` | K | [ecs-services](ledger/ecs-services.md) |
| component | crates/boyko_ecs/src/ecs/core/asset/staging.rs:58 `queue: Vec<Staged<A>>,` | AssetStaging&lt;A>::queue | `Vec<Staged<A> { handle, cpu: A::Cpu } (not Copy)>` | R | [ecs-services](ledger/ecs-services.md) |
| component | D:/wt/ui:crates/boyko_ui/src/sprite.rs:257 `sheets: Vec<UiSheet>,` | UiSheetTable::sheets | `Vec<UiSheet>` | R | [ui-lane](ledger/ui-lane.md) |
| component | D:/wt/ui:crates/boyko_ui/src/text/font.rs:139 `fonts: Vec<FontEntry>,` | FontTable::fonts | `Vec<FontEntry>` | R | [ui-lane](ledger/ui-lane.md) |
| dense-component | crates/boyko_render/src/asset_refcount.rs:556 `scratch: &mut Vec<FreeEntry>,` | retire_deferred_frees(scratch) - backing field is boyko_app HostState::retire_scratch (boyko_app/src/host.rs:96) | `Vec<FreeEntry>` | F | [render](ledger/render.md) |
| dense-component | crates/boyko_scene/src/asset_refs.rs:149 `entries: Vec<FreeEntry>,` | DeferredFree::entries | `Vec<FreeEntry>` | R | [physics-scene-math](ledger/physics-scene-math.md) |

### light (10 rows)

**Light.** **enable-state** `LightEnabled`, declared default-enabled (KF-17). The 8 cached id-collecting systems (render `light_system.rs:729-751`) exist only because a never-toggled bitset row reads DISABLED ("/// A never-toggled row reads DISABLED (the bitset default). To keep pre-existing /", boyko_render/src/light.rs:244). With KF-17 they disappear. Two further rows, the boot images of the empty light-table header, are resource-column seeds (gap 4). KF-17 is the engine design's EK4.

| form | row | owner | container&lt;elem&gt; | class | group |
|---|---|---|---|---|---|
| enable-state | crates/boyko_render/src/light_system.rs:729 `q.iter_entities().map(\|(id, _)\| id).collect::<Vec<_>>()` | light_seed_state -> LightSeedState::added_dir closure (System::Out = Vec&lt;EntityId>) | `Vec<EntityId>` | F | [render](ledger/render.md) |
| enable-state | crates/boyko_render/src/light_system.rs:733 `q.iter_entities().map(\|(id, _)\| id).collect::<Vec<_>>()` | light_seed_state -> LightSeedState::added_sky closure (System::Out = Vec&lt;EntityId>) | `Vec<EntityId>` | F | [render](ledger/render.md) |
| enable-state | crates/boyko_render/src/light_system.rs:736 `q.iter_entities().map(\|(id, _)\| id).collect::<Vec<_>>()` | light_seed_state -> LightSeedState::added_point closure (System::Out = Vec&lt;EntityId>) | `Vec<EntityId>` | F | [render](ledger/render.md) |
| enable-state | crates/boyko_render/src/light_system.rs:739 `q.iter_entities().map(\|(id, _)\| id).collect::<Vec<_>>()` | light_seed_state -> LightSeedState::added_spot closure (System::Out = Vec&lt;EntityId>) | `Vec<EntityId>` | F | [render](ledger/render.md) |
| enable-state | crates/boyko_render/src/light_system.rs:742 `q.iter_entities().map(\|(id, _)\| id).collect::<Vec<_>>()` | light_seed_state -> LightSeedState::all_dir closure (System::Out = Vec&lt;EntityId>) | `Vec<EntityId>` | B | [render](ledger/render.md) |
| enable-state | crates/boyko_render/src/light_system.rs:745 `q.iter_entities().map(\|(id, _)\| id).collect::<Vec<_>>()` | light_seed_state -> LightSeedState::all_sky closure (System::Out = Vec&lt;EntityId>) | `Vec<EntityId>` | B | [render](ledger/render.md) |
| enable-state | crates/boyko_render/src/light_system.rs:748 `q.iter_entities().map(\|(id, _)\| id).collect::<Vec<_>>()` | light_seed_state -> LightSeedState::all_point closure (System::Out = Vec&lt;EntityId>) | `Vec<EntityId>` | B | [render](ledger/render.md) |
| enable-state | crates/boyko_render/src/light_system.rs:751 `q.iter_entities().map(\|(id, _)\| id).collect::<Vec<_>>()` | light_seed_state -> LightSeedState::all_spot closure (System::Out = Vec&lt;EntityId>) | `Vec<EntityId>` | B | [render](ledger/render.md) |
| resource-column | crates/boyko_app/src/gpu_scene/mod.rs:698 `fn pack_light_table(header: &LightHeaderGpu, lights: &[GpuLight]) -> Vec<u32> {` | gpu_scene::pack_light_table | `Vec<u32>` | B | [app-demo](ledger/app-demo.md) |
| resource-column | crates/boyko_app/src/gpu_scene/mod.rs:699 `let mut words = vec![0u32; LIGHT_HEADER_BASE_WORDS + lights.len() * GPU_LIGHT_WORDS];` | gpu_scene::pack_light_table::words | `Vec<u32>` | B | [app-demo](ledger/app-demo.md) |

### widget (20 rows)

**Widget.** Rows (ui-lane is the census of record for boyko_ui in rev 2) imply:
- **components** `UiInstance` (render `ui/pack.rs:801` on the ui-lane tree, which supersedes joltab `:143` in rev 3), the pack-input copy `UiNode` (lane `ui/gather.rs:340` and `ui/upload.rs:203`, which supersede joltab `ui/upload.rs:258`) and the `UiNameStr` name;
- **markers** `UiRoot` and `UiDocRoot`, which replace the cross-frame root copies;
- **relation** `Children`: the `UiTreeView` parallel copy and `LiveNode::children` are deleted once KF-28 lets the reconcile read the ECS in place (the lane measured the copy as a defect generator);
- **events**: `hover_entered` in its trigger form (engine ED9, writer change W5; KF-24 is not built) and the lane's tween completion, which becomes `Commands::remove` from the tick (UL-D7; the engine design reached the same answer: main:docs/unification/ENGINE-RUNTIME-ECS-DESIGN.md:336 "- Tween completions become `EntityCommands::remove` (X-4).");
- **system-scratch** for the four UI gathers, which fill a host Vec before the mapped ring (gap 4), and for the exclusive systems' scratch held through KF-44 (gap 7).

| form | row | owner | container&lt;elem&gt; | class | group |
|---|---|---|---|---|---|
| component | D:/wt/ui:crates/boyko_render/src/ui/gather.rs:340 `node_buf: &mut Vec<UiNode>,` | gather_ui_nodes(node_buf) | `Vec<UiNode>` | F | [ui-lane](ledger/ui-lane.md) |
| component | D:/wt/ui:crates/boyko_render/src/ui/pack.rs:784 `sink: &mut Vec<UiInstance>,` | emit_ui_node_records(sink) | `Vec<UiInstance>` | F | [ui-lane](ledger/ui-lane.md) |
| component | D:/wt/ui:crates/boyko_render/src/ui/pack.rs:801 `pub pack: Vec<UiInstance>,` | UiRenderScratch::pack | `Vec<UiInstance>` | F | [ui-lane](ledger/ui-lane.md) |
| component | D:/wt/ui:crates/boyko_render/src/ui/pack.rs:832 `pack: Vec::new(),` | UiRenderScratch::default (pack: Vec::new()) | `Vec<UiInstance>` | B | [ui-lane](ledger/ui-lane.md) |
| component | D:/wt/ui:crates/boyko_render/src/ui/upload.rs:203 `node_buf: Vec<UiNode>,` | UiUploadSystem::node_buf | `Vec<UiNode>` | F | [ui-lane](ledger/ui-lane.md) |
| component | D:/wt/ui:crates/boyko_render/src/ui/upload.rs:262 `node_buf: Vec::new(),` | UiUploadSystem::new (node_buf: Vec::new()) | `Vec<UiNode>` | B | [ui-lane](ledger/ui-lane.md) |
| component | D:/wt/ui:crates/boyko_ui/src/reload/state.rs:31 `spill: Vec<Entity>,` | SmallRoots::spill (in UiHotReload::doc_roots) | `Vec<Entity>` | B | [ui-lane](ledger/ui-lane.md) |
| component | D:/wt/ui:crates/boyko_ui/src/reload/tree_view.rs:81 `pub nodes: Vec<LiveNode>,` | UiTreeView::nodes | `Vec<LiveNode>` | B | [ui-lane](ledger/ui-lane.md) |
| component | D:/wt/ui:crates/boyko_ui/src/reload/tree_view.rs:89 `let mut nodes = Vec::new();` | UiTreeView::build (nodes) | `Vec<LiveNode>` | B | [ui-lane](ledger/ui-lane.md) |
| component | D:/wt/ui:crates/boyko_ui/src/resources.rs:254 `pub(crate) roots: Vec<Entity>,` | LayoutScratch::roots | `Vec<Entity>` | R | [ui-lane](ledger/ui-lane.md) |
| component | D:/wt/ui:crates/boyko_ui/src/resources.rs:338 `roots: Vec::with_capacity(SEED_ROOTS),` | LayoutScratch::with_seeds | `Vec<field element>` | B | [ui-lane](ledger/ui-lane.md) |
| component | D:/wt/ui:crates/boyko_ui/src/text/ast.rs:50 `pub text: String,` | UiNameStr::text | `String<u8>` | B | [ui-lane](ledger/ui-lane.md) |
| component | D:/wt/ui:crates/boyko_ui/src/text/ast.rs:56 `pub(crate) fn new(text: String) -> Self {` | UiNameStr::new | `String<u8>` | B | [ui-lane](ledger/ui-lane.md) |
| component | D:/wt/ui:crates/boyko_ui/src/text/parser.rs:289 `(Some(UiNameStr::new(name.to_string())), rest)` | text::parser::split_name (UiNameStr) | `String<u8>` | B | [ui-lane](ledger/ui-lane.md) |
| event | D:/wt/ui:crates/boyko_ui/src/animation.rs:473 `done: Vec<(EntityId, ComponentId)>,` | UiTweenScratch::done | `Vec<(EntityId, ComponentId)>` | F | [ui-lane](ledger/ui-lane.md) |
| event | D:/wt/ui:crates/boyko_ui/src/interaction/focus.rs:128 `pub hover_entered: Vec<Entity>,` | UiInteractionScratch::hover_entered | `Vec<Entity>` | F | [ui-lane](ledger/ui-lane.md) |
| relation | D:/wt/ui:crates/boyko_ui/src/reload/reconcile.rs:332 `.map(\|n\| n.children.clone())` | reconcile::reconcile_children (live_children_of) | `Vec<Entity>` | B | [ui-lane](ledger/ui-lane.md) |
| relation | D:/wt/ui:crates/boyko_ui/src/reload/tree_view.rs:40 `pub children: Vec<Entity>,` | LiveNode::children | `Vec<Entity>` | B | [ui-lane](ledger/ui-lane.md) |
| relation | D:/wt/ui:crates/boyko_ui/src/reload/tree_view.rs:100 `let children: Vec<Entity> = world` | UiTreeView::build (children) | `Vec<Entity>` | B | [ui-lane](ledger/ui-lane.md) |
| relation | D:/wt/ui:crates/boyko_ui/src/reload/tree_view.rs:107 `children: children.clone(),` | UiTreeView::build (children.clone()) | `Vec<Entity>` | B | [ui-lane](ledger/ui-lane.md) |

### text-run (3 rows)

**Text run.** **system-scratch** glyph lanes (`boyko_ui/src/text/emit.rs:65/89/141`), which need KF-01.

| form | row | owner | container&lt;elem&gt; | class | group |
|---|---|---|---|---|---|
| system-scratch | D:/wt/ui:crates/boyko_ui/src/text/emit.rs:65 `pub glyphs: Vec<GlyphInstance>,` | TextEmitScratch::glyphs | `Vec<GlyphInstance>` | F | [ui-lane](ledger/ui-lane.md) |
| system-scratch | D:/wt/ui:crates/boyko_ui/src/text/emit.rs:89 `pub fn emit_node(node: &TextNode, fonts: &FontTable, out: &mut Vec<GlyphInstance>) {` | text::emit::emit_node (pub) | `Vec<GlyphInstance>` | F | [ui-lane](ledger/ui-lane.md) |
| system-scratch | D:/wt/ui:crates/boyko_ui/src/text/emit.rs:141 `out: &mut Vec<GlyphInstance>,` | text::emit::emit_glyphs (pub) | `Vec<GlyphInstance>` | F | [ui-lane](ledger/ui-lane.md) |

### window (13 rows)

**Window** (a real entity type in rev 2: gap 8, KF-46, engine Q3 main:docs/unification/ENGINE-RUNTIME-ECS-DECISIONS.md:25 "| Q3 | Multiplicity in v1 | **(b) Window and player are entities now** | no - the design recommended (a) |"). One entity per OS window. Per-window data are components on it: the window state (`window.rs:230`), the frame driver (`frame_driver.rs:48`), the swapchain images and views (`swapchain.rs:64/66`). The OS holds a pointer to the input ring, so the ring (`window.rs:122`, `:365`) is a **dense-component**: its slot never moves. Raw-input **event** lanes carry the window entity. Surface and format enumeration stay **system-scratch**.

| form | row | owner | container&lt;elem&gt; | class | group |
|---|---|---|---|---|---|
| component | crates/boyko_rhi_vulkan/src/present/frame_driver.rs:48 `pub(crate) render_finished: Vec<VkSemaphore>,` | Renderer::render_finished | `Vec<VkSemaphore>` | R | [rhi](ledger/rhi.md) |
| component | crates/boyko_rhi_vulkan/src/present/swapchain.rs:64 `pub(crate) images: Vec<VkImage>,` | Swapchain::images | `Vec<VkImage>` | X | [rhi](ledger/rhi.md) |
| component | crates/boyko_rhi_vulkan/src/present/swapchain.rs:66 `pub(crate) image_views: Vec<VkImageView>,` | Swapchain::image_views | `Vec<VkImageView>` | R | [rhi](ledger/rhi.md) |
| component | crates/boyko_rhi_vulkan/src/window.rs:230 `class_name: Vec<u16>,` | Window::class_name | `Vec<u16 (NUL-terminated UTF-16 class name)>` | B | [rhi](ledger/rhi.md) |
| dense-component | crates/boyko_rhi_vulkan/src/window.rs:122 `buf: Box<[CapturedMsg]>,` | InputRing::buf | `Box<[T]><CapturedMsg>` | R | [rhi](ledger/rhi.md) |
| dense-component | crates/boyko_rhi_vulkan/src/window.rs:365 `let input_ring = Box::into_raw(Box::new(InputRing::with_capacity(INPUT_RING_CAP)));` | Window::open (owned via Window::input_ring raw pointer) | `Box<T><InputRing>` | R | [rhi](ledger/rhi.md) |
| event | crates/boyko_input/src/raw/queue.rs:35 `buf: Box<[RawInputEvent]>,` | RawInputQueue::buf | `Box<[T]><RawInputEvent>` | R | [ui-input](ledger/ui-input.md) |
| event | crates/boyko_input/src/raw/queue.rs:59 `let buf = vec![filler; cap].into_boxed_slice();` | RawInputQueue::with_capacity | `Box<[T]><RawInputEvent>` | B | [ui-input](ledger/ui-input.md) |
| system-scratch | crates/boyko_rhi_vulkan/src/present/surface.rs:180 `let mut formats = vec![VkSurfaceFormatKhr { format: 0, color_space: 0 }; count as usize];` | surface::pick_surface_format | `Vec<VkSurfaceFormatKHR>` | B | [rhi](ledger/rhi.md) |
| system-scratch | crates/boyko_rhi_vulkan/src/present/surface.rs:238 `let mut modes = vec![0i32; count as usize];` | surface::present_mode_supported | `Vec<i32 (VkPresentModeKHR)>` | F | [rhi](ledger/rhi.md) |
| system-scratch | crates/boyko_rhi_vulkan/src/window.rs:274 `let class_name = to_wide(&format!("boyko_rhi_vulkan_window_class_{class_id}"));` | Window::open | `String<u8>` | B | [rhi](ledger/rhi.md) |
| system-scratch | crates/boyko_rhi_vulkan/src/window.rs:352 `if std::env::var_os("BOYKO_WIN_HIDDEN").is_none() {` | Window::open | `other:OsString<u16 env value (discarded)>` | B | [rhi](ledger/rhi.md) |
| system-scratch | crates/boyko_rhi_vulkan/src/window.rs:761 `fn to_wide(s: &str) -> Vec<u16> {` | window::to_wide | `Vec<u16 (NUL-terminated UTF-16)>` | B | [rhi](ledger/rhi.md) |

### observer (5 rows)

**Observer** (a NEW entity type introduced by this ledger). An **Observer** component (runner, key, component) sits on an observer entity. Entity-targeted observers link to the observed entity through an **Observes relation**, whose despawn cascade replaces the generation recycle guard (entity_store.rs:96-97). This covers 5 rows in `observers/entity_store.rs` and needs KF-15. **Decided in rev 2** (section Decisions): observers are entities; the global dispatch tables become derived contiguous indexes.

| form | row | owner | container&lt;elem&gt; | class | group |
|---|---|---|---|---|---|
| relation | crates/boyko_ecs/src/ecs/core/component/observers/entity_store.rs:102 `entries: Vec<EntityObserverEntry>,` | EntityObserverList::entries | `Vec<EntityObserverEntry (Copy: DispatchKey, ComponentId, ObserverId, fn-ptr runner)>` | E | [ecs-storage](ledger/ecs-storage.md) |
| relation | crates/boyko_ecs/src/ecs/core/component/observers/entity_store.rs:111 `inner: Option<Box<EntityObserverInner>>,` | EntityObserverStore::inner | `Box<T><EntityObserverInner>` | R | [ecs-storage](ledger/ecs-storage.md) |
| relation | crates/boyko_ecs/src/ecs/core/component/observers/entity_store.rs:120 `by_entity: SparseMap<u32>,` | EntityObserverInner::by_entity | `other:SparseMap (boyko_utils; 3 x Vec inside)<u32 (arena handle)>` | K | [ecs-storage](ledger/ecs-storage.md) |
| relation | crates/boyko_ecs/src/ecs/core/component/observers/entity_store.rs:123 `arena: Vec<EntityObserverList>,` | EntityObserverInner::arena | `Vec<EntityObserverList>` | K | [ecs-storage](ledger/ecs-storage.md) |
| relation | crates/boyko_ecs/src/ecs/core/component/observers/entity_store.rs:125 `free_list: Vec<u32>,` | EntityObserverInner::free_list | `Vec<u32 (free arena handle)>` | K | [ecs-storage](ledger/ecs-storage.md) |

### prefab (4 rows)

**Prefab** (a NEW entity type). Templates are entities with a default-excluded marker (KF-13). Capture and instantiate both become `clone_subtree`, which closes the dense-membership gap noted at prefab.rs:44-45. This covers 4 rows here and 8 prefab rows under KF-13. **Decided in rev 2**: yes; this is the engine design's EK18.

| form | row | owner | container&lt;elem&gt; | class | group |
|---|---|---|---|---|---|
| component | crates/boyko_ecs/src/ecs/core/clone/prefab.rs:136 `struct RawBlob {` | Prefab::blob (RawBlob) | `other:RawBlob (hand-rolled std::alloc alloc/realloc/dealloc byte arena)<u8 (captured component values, max-aligned)>` | R | [ecs-storage](ledger/ecs-storage.md) |
| component | crates/boyko_ecs/src/ecs/core/clone/prefab.rs:281 `nodes: Vec<PrefabNode>,` | Prefab::nodes | `Vec<PrefabNode>` | R | [ecs-storage](ledger/ecs-storage.md) |
| component | crates/boyko_ecs/src/ecs/core/clone/prefab.rs:284 `components: Vec<PrefabComponent>,` | Prefab::components | `Vec<PrefabComponent>` | R | [ecs-storage](ledger/ecs-storage.md) |
| component | crates/boyko_ecs/src/ecs/core/clone/prefab.rs:364 `committed: Vec<PrefabComponent>,` | BlobGuard::committed | `Vec<PrefabComponent>` | B | [ecs-storage](ledger/ecs-storage.md) |

### system (374 rows)

**System** (including system sets and condition systems, because the closed list has no set entity). This entity type exists only under KF-14 (system entities). Rows imply:
- a **dense-component** `SystemBox`: the executor caches row addresses, so the slots must never move;
- **components** `GpuAccessIntent`, the descriptor and set names;
- **relations** `InSet`, `Before`/`After` and `RunIf`;
- the system's **event** output: the `CommandQueue` byte channel and the event lanes it writes;
- the per-system **system-scratch**, which is most of this entity's rows.

**Decided in rev 2** (section Decisions): systems are hidden entities; the compiled executor tables stay kernel-internal, so the per-frame dispatch path does not change. The fallback recorded on every row (kernel-internal on Schedule-owned storage) is what the overturn gate would restore.

By group and form: app-demo system-scratch 30, codec-tools system-scratch 38, ecs-schedule component 6, ecs-schedule dense-component 2, ecs-schedule kernel-internal 19, ecs-schedule relation 18, ecs-schedule system-scratch 8, ecs-services event 7, ecs-services system-scratch 8, ecs-storage system-scratch 18, physics-scene-math event 1, physics-scene-math system-scratch 43, render resource-column 8, render system-scratch 34, rhi system-scratch 34, ui-input system-scratch 14, ui-lane system-scratch 86.

The rows in entity forms on this owner:

| form | row | owner | container&lt;elem&gt; | class | group |
|---|---|---|---|---|---|
| component | crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs:106 `pub(crate) descriptors: Vec<SystemDescriptor>,` | ScheduleBuilder::descriptors | `Vec<SystemDescriptor>` | B | [ecs-schedule](ledger/ecs-schedule.md) |
| component | crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs:131 `pub(crate) set_names: HashMap<SystemSetId, &'static str>,` | ScheduleBuilder::set_names | `HashMap<SystemSetId -> &'static str>` | B | [ecs-schedule](ledger/ecs-schedule.md) |
| component | crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs:153 `descriptors: Vec::new(),` | ScheduleBuilder::new | `Vec<SystemDescriptor>` | B | [ecs-schedule](ledger/ecs-schedule.md) |
| component | crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs:158 `set_names: HashMap::new(),` | ScheduleBuilder::new | `HashMap<SystemSetId -> &'static str>` | B | [ecs-schedule](ledger/ecs-schedule.md) |
| component | crates/boyko_ecs/src/ecs/core/system/system_meta.rs:140 `pub(crate) gpu_intent: Option<Box<GpuAccessIntent>>,` | SystemMeta::gpu_intent | `Box<T><GpuAccessIntent>` | K | [ecs-schedule](ledger/ecs-schedule.md) |
| component | crates/boyko_ecs/src/ecs/core/system/system_meta.rs:331 `self.gpu_intent = Some(Box::new(intent));` | SystemMeta::set_gpu_intent | `Box<T><GpuAccessIntent>` | B | [ecs-schedule](ledger/ecs-schedule.md) |
| dense-component | crates/boyko_ecs/src/ecs/core/schedule/schedule.rs:122 `pub(crate) systems: Vec<SystemBox>,` | Schedule::systems | `Vec<SystemBox>` | K | [ecs-schedule](ledger/ecs-schedule.md) |
| dense-component | crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs:682 `let mut systems: Vec<SystemBox> = Vec::with_capacity(n_final);` | ScheduleBuilder::try_build | `Vec<SystemBox>` | B | [ecs-schedule](ledger/ecs-schedule.md) |
| event | crates/boyko_ecs/src/ecs/core/commands/command_queue.rs:83 `pub(crate) bytes: Vec<MaybeUninit<u8>>,` | CommandQueue::bytes | `Vec<MaybeUninit<u8> (packed [CommandMeta][payload] records)>` | R | [ecs-services](ledger/ecs-services.md) |
| event | crates/boyko_ecs/src/ecs/core/commands/command_queue.rs:85 `pub(crate) panic_recovery: Vec<MaybeUninit<u8>>,` | CommandQueue::panic_recovery | `Vec<MaybeUninit<u8> (un-run tail of a panicked apply)>` | R | [ecs-services](ledger/ecs-services.md) |
| event | crates/boyko_ecs/src/ecs/core/events/erased_buffer.rs:109 `data: Vec<MaybeUninit<u8>>,` | ErasedKindBuffer&lt;K>::data (aliases ParametersBuffer / ParticipantBuffer) | `Vec<MaybeUninit<u8> (packed Copy element sets)>` | R | [ecs-services](ledger/ecs-services.md) |
| event | crates/boyko_ecs/src/ecs/core/events/event_buffer.rs:119 `pub(crate) write_buf: UnsafeCell<Box<[MaybeUninit<E>]>>,` | ThreadLaneWriter&lt;E>::write_buf | `Box<[T]><MaybeUninit<E> (capacity_per_lane slots)>` | R | [ecs-services](ledger/ecs-services.md) |
| event | crates/boyko_ecs/src/ecs/core/events/event_buffer.rs:238 `pub(crate) reader_buf: Box<[MaybeUninit<E>]>,` | EventBuffer&lt;E>::reader_buf | `Box<[T]><MaybeUninit<E> (thread_count * capacity_per_lane)>` | R | [ecs-services](ledger/ecs-services.md) |
| event | crates/boyko_ecs/src/ecs/core/events/event_buffer.rs:243 `pub(crate) lanes: Box<[ThreadLanePair<E>]>,` | EventBuffer&lt;E>::lanes | `Box<[T]><ThreadLanePair<E> (128 B, align 64)>` | R | [ecs-services](ledger/ecs-services.md) |
| event | crates/boyko_ecs/src/ecs/core/events/event_dispatcher.rs:221 `let buffer = Box::new(EventBuffer::<E>::new(cfg)?);` | EventDispatcher::preregister::&lt;E> | `Box<T><EventBuffer<E>>` | R | [ecs-services](ledger/ecs-services.md) |
| event | crates/boyko_scene/src/propagation.rs:133 `detached: Vec<Entity>,` | TransformPropagationScratch::detached | `Vec<Entity>` | R | [physics-scene-math](ledger/physics-scene-math.md) |
| relation | crates/boyko_ecs/src/ecs/core/schedule/schedule.rs:161 `pub(crate) system_conditions: Vec<Vec<BoolSystem>>,` | Schedule::system_conditions | `Vec<Vec<Box<dyn System<Out = bool>>>>` | K | [ecs-schedule](ledger/ecs-schedule.md) |
| relation | crates/boyko_ecs/src/ecs/core/schedule/schedule.rs:172 `pub(crate) set_conditions: Vec<SetConditionEntry>,` | Schedule::set_conditions | `Vec<SetConditionEntry>` | K | [ecs-schedule](ledger/ecs-schedule.md) |
| relation | crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs:116 `pub(crate) set_members: HashMap<SystemSetId, Vec<SystemKey>>,` | ScheduleBuilder::set_members | `HashMap<SystemSetId -> Vec<SystemKey>>` | B | [ecs-schedule](ledger/ecs-schedule.md) |
| relation | crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs:122 `pub(crate) set_ordering: Vec<SetOrderEdge>,` | ScheduleBuilder::set_ordering | `Vec<SetOrderEdge>` | B | [ecs-schedule](ledger/ecs-schedule.md) |
| relation | crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs:127 `pub(crate) set_parents: HashMap<SystemSetId, Vec<SystemSetId>>,` | ScheduleBuilder::set_parents | `HashMap<SystemSetId -> Vec<SystemSetId>>` | B | [ecs-schedule](ledger/ecs-schedule.md) |
| relation | crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs:139 `pub(crate) set_conditions: HashMap<SystemSetId, Vec<BoolSystem>>,` | ScheduleBuilder::set_conditions | `HashMap<SystemSetId -> Vec<Box<dyn System<Out = bool>>>>` | B | [ecs-schedule](ledger/ecs-schedule.md) |
| relation | crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs:155 `set_members: HashMap::new(),` | ScheduleBuilder::new | `HashMap<SystemSetId -> Vec<SystemKey>>` | B | [ecs-schedule](ledger/ecs-schedule.md) |
| relation | crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs:156 `set_ordering: Vec::new(),` | ScheduleBuilder::new | `Vec<SetOrderEdge>` | B | [ecs-schedule](ledger/ecs-schedule.md) |
| relation | crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs:157 `set_parents: HashMap::new(),` | ScheduleBuilder::new | `HashMap<SystemSetId -> Vec<SystemSetId>>` | B | [ecs-schedule](ledger/ecs-schedule.md) |
| relation | crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs:159 `set_conditions: HashMap::new(),` | ScheduleBuilder::new | `HashMap<SystemSetId -> Vec<BoolSystem>>` | B | [ecs-schedule](ledger/ecs-schedule.md) |
| relation | crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs:683 `let mut system_conditions: Vec<Vec<BoolSystem>> = Vec::with_capacity(n_final);` | ScheduleBuilder::try_build | `Vec<Vec<Box<dyn System<Out = bool>>>>` | B | [ecs-schedule](ledger/ecs-schedule.md) |
| relation | crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs:693 `let mut set_conditions_table: Vec<SetConditionEntry> = Vec::new();` | ScheduleBuilder::try_build | `Vec<SetConditionEntry>` | B | [ecs-schedule](ledger/ecs-schedule.md) |
| relation | crates/boyko_ecs/src/ecs/core/schedule/system_descriptor.rs:50 `pub(crate) ordering_hints: Vec<OrderingEdge>,` | SystemDescriptor::ordering_hints | `Vec<OrderingEdge>` | B | [ecs-schedule](ledger/ecs-schedule.md) |
| relation | crates/boyko_ecs/src/ecs/core/schedule/system_descriptor.rs:55 `pub(crate) sets: Vec<SystemSetId>,` | SystemDescriptor::sets | `Vec<SystemSetId>` | B | [ecs-schedule](ledger/ecs-schedule.md) |
| relation | crates/boyko_ecs/src/ecs/core/schedule/system_descriptor.rs:62 `pub(crate) conditions: Vec<BoolSystem>,` | SystemDescriptor::conditions | `Vec<Box<dyn System<Out = bool>>>` | B | [ecs-schedule](ledger/ecs-schedule.md) |
| relation | crates/boyko_ecs/src/ecs/core/schedule/system_descriptor.rs:80 `ordering_hints: Vec::new(),` | SystemDescriptor::new | `Vec<OrderingEdge>` | B | [ecs-schedule](ledger/ecs-schedule.md) |
| relation | crates/boyko_ecs/src/ecs/core/schedule/system_descriptor.rs:81 `sets: Vec::new(),` | SystemDescriptor::new | `Vec<SystemSetId>` | B | [ecs-schedule](ledger/ecs-schedule.md) |
| relation | crates/boyko_ecs/src/ecs/core/schedule/system_descriptor.rs:82 `conditions: Vec::new(),` | SystemDescriptor::new | `Vec<Box<dyn System<Out = bool>>>` | B | [ecs-schedule](ledger/ecs-schedule.md) |

### schedule (102 rows)

**Schedule.** The compiled executor tables, the executor scratch, the completion ring and the build scratch are **kernel-internal**. The `insert_state` closures become deferred world mutations, i.e. Commands (**event**). The per-command transients of the kernel apply window (migration, clone, prefab, trigger DFS) are **system-scratch** through KF-05.

By group and form: app-demo kernel-internal 1, ecs-schedule event 4, ecs-schedule kernel-internal 75, ecs-services kernel-internal 2, ecs-storage system-scratch 20.

The rows in entity forms on this owner:

| form | row | owner | container&lt;elem&gt; | class | group |
|---|---|---|---|---|---|
| event | crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs:92 `insert: Box<dyn FnOnce(&mut EcsMaster)>,` | StateRegistration::insert | `Box<dyn><dyn FnOnce(&mut EcsMaster)>` | B | [ecs-schedule](ledger/ecs-schedule.md) |
| event | crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs:144 `state_registrations: Vec<StateRegistration>,` | ScheduleBuilder::state_registrations | `Vec<StateRegistration>` | B | [ecs-schedule](ledger/ecs-schedule.md) |
| event | crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs:160 `state_registrations: Vec::new(),` | ScheduleBuilder::new | `Vec<StateRegistration>` | B | [ecs-schedule](ledger/ecs-schedule.md) |
| event | crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs:263 `insert: Box::new(move \|world: &mut EcsMaster\| world.insert_state::<S>(initial)),` | ScheduleBuilder::insert_state | `Box<dyn><closure capturing S>` | B | [ecs-schedule](ledger/ecs-schedule.md) |

### pool (42 rows)

**Pool.** Thread-pool storage is **kernel-internal**: `PoolInner` tables, lanes and handles. Task cells and scope state are **scope-arena**. The pool sits BELOW every World (`boyko_ecs` depends on `boyko_threadpool`), so every per-entity form is refuted for it. The ECS answer for the pool is to unify on the same memory library (KF-32 / KF-33 / KF-34), not to turn the pool into components. Rev 2 adds the 12 `thread_local!` statics (class T, kernel-internal): they become fields of one engine thread-context column (KF-45), reached by slot, because on windows-gnu every `thread_local!` read costs two contended RMWs plus `FlsSetValue` and one `System` allocation per thread per key.

By group and form: app-demo kernel-internal 3, ecs-schedule kernel-internal 3, ecs-schedule scope-arena 6, ecs-services kernel-internal 1, pool-utils-log diagnostics 2, pool-utils-log kernel-internal 23, pool-utils-log scope-arena 4.

### relation-endpoint (3 rows)

**Any entity, as a relation endpoint.** Every one-to-many relation target, `Children` included, keeps its reverse index on kernel storage (KF-11). **Decided in rev 2**: as per-target spans of a K7 segmented column (engine EK15c), not as intrusive links: traversal (transform propagation, UI layout, joint cleanup) reads one contiguous span instead of chasing one link per child. Covers 3 rows (`hierarchy/mod.rs:120/159`, `relationship/collection.rs:85`).

| form | row | owner | container&lt;elem&gt; | class | group |
|---|---|---|---|---|---|
| relation | crates/boyko_ecs/src/ecs/core/hierarchy/mod.rs:120 `pub struct Children(Vec<Entity>);` | Children.0 | `Vec<Entity>` | E | [ecs-services](ledger/ecs-services.md) |
| relation | crates/boyko_ecs/src/ecs/core/hierarchy/mod.rs:159 `Self(vec![child])` | Children::with_one | `Vec<Entity>` | E | [ecs-services](ledger/ecs-services.md) |
| relation | crates/boyko_ecs/src/ecs/core/relationship/collection.rs:85 `Vec::with_capacity(cap)` | &lt;Vec&lt;Entity> as RelationshipSourceCollection>::with_capacity | `Vec<Entity>` | E | [ecs-services](ledger/ecs-services.md) |

### none (1682 rows)

**none.** Singleton tables (resource-column), the kernel's own bookkeeping (kernel-internal) and out-of-scope rows. The builder of every group asserted that `none` appears only with these forms.

Forms: out-of-scope:compile-time 1021, diagnostics 299, resource-column 144, kernel-internal 137, out-of-scope:test-only 40, out-of-scope:os-owned 31, out-of-scope:third-party 10.

### The physics entity model (physics-scene-math group, with rev-2 notes)

**Body identity (SolverScratch.bodies gather, BodyIndex)** (rows: none (already ScratchColumn); decides rows 0-1)

- *Entity model:* A body IS an entity. BodyIndex is its position in the archetype-row concatenation of one gather, valid within one pass only (boyko_physics/src/systems.rs:1118 "Correct UNDER the "no structural change between gather and apply" invariant:"). Anything keyed by BodyIndex across frames (IslandSleep's latch today) is keyed by something that is not an identity; a swap_remove (boyko_ecs/src/ecs/core/archetype/archetype.rs:1268 "self.entity_ids.swap_remove(removed_unit_index.0);") renames rows.
- *ECS form:* component (Table RigidBody / RigidBodyMass / Collider) + system-scratch gather mirror, which both plans keep as a cache optimisation (D:/wt/joltab/docs/ARCH-AUDIT-ECS-DATA-REMEDIATION.md:24 "**keep the gather, but move its buffer + all physics bulk onto `ComponentPool`**"). After Stage P the dense slot becomes the stable per-body index (D:/wt/joltab/docs/DENSE-COMPONENTS-PLAN.md:19 "Live slots never move."), with determinism downgraded to a fixed op sequence (D:/wt/joltab/docs/DENSE-COMPONENTS-PLAN.md:56 "coloring DEPENDS on absolute body slot values").
- *Hot-loop cost:* Unchanged by this ledger. The gather turns scattered body_a/body_b reads into a dense mirror; per-body durable extras (sleep) ride the same walk at 2-3 bytes per row.
- *Missing kernel feature:* KR-1 row->entity datum (D:/claude/BoykoEngine/docs/physics/ADVANCED-PHYSICS-DESIGN-SPACE.md:183 "**Kernel request KR-1**: an entity datum in `Query`") for any cross-frame per-body data that is not a component; KF-enable-write-in-iteration for per-row bits.
- *Rev 2:* [rev2] Decided by physics D1 and Q5: identity is a stable dense-group slot, not the archetype row; `RigidBody` stays the authored table component and the solver works on the derived PhysicsBody group. main:docs/physics/PHYSICS-ECS-UNIFICATION-DECISIONS.md:33 "\| Q5 \| Where pose and velocity live \| **(a) `RigidBody` stays an authored table component; the solver works on the derived dense group; writeback touches only awake dynamic bodies** \| SP-1 at rung R0: if (b) is faster beyond the band at W=8 on BOTH the pyramid and a 20k-body scene, revisit before U4 \|"

**IslandSleep** (rows: rows 0-7)

- *Entity model:* Latch and debounce are BODY state (entity); frozen/energy are per-ISLAND per-step derivations. An island is not an entity: ids are volatile (boyko_physics/src/resources.rs:3008 "Island ids are NOT stable: [`ConstraintGraph::build`] re-derives them every frame") and merge/split has no surviving identity.
- *ECS form:* asleep -> enable-state Sleeping (plan D:/wt/joltab/docs/ARCH-AUDIT-ECS-DATA-REMEDIATION.md:30 "sleep → `Sleeping` component + `EnableColumn` paged-bitset tag (O(1) flip, no migration churn)"); below_count -> component; frozen_islands/energy -> system-scratch.
- *Hot-loop cost:* Solver: none - it reads the per-step awake mask. Gather: +1 paged bit test and +2 B per row. Apply: a bit write only on a latch flip. Removes a latent defect: today the latch follows the row, not the body.
- *Missing kernel feature:* KF-enable-write-in-iteration; KF-dense-enable-iteration only if bodies go dense.
- *Rev 2:* [rev2] [rev2 writer, physics decision Q3 / design D4] ecs_form dense-component: the latch and its debounce counter are the `asleep` and `below` fields of `BodyGate`, a 4 B untracked column of the PhysicsBody dense group (K3), co-slotted with the solver columns and addressed by the body's stable slot (D1). main:docs/physics/PHYSICS-ECS-UNIFICATION-DECISIONS.md:31 "\| Q3 \| How gameplay sees sleep \| **(a) `BodyGate` in the group, plus sleep/wake transition events** \| None expected; gated by frozen-body byte-identity and the census \|" ; main:docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md:304 "\| `BodyGate` {flags, asleep, below} \| group column, untracked \| body \| D4. Replaces `resources.rs:3057` `asleep: Vec<bool>,` and `:3062` `below_count: Vec<u16>,` \|". The rev-1 form (an EnableTag `Sleeping` plus a `SleepCounter` table component) is withdrawn: an EnableTag lives at (archetype, row), so the solver would pay a random lookup per body per pass or keep a synced copy, and a toggle needs &mut EcsMaster mid-step (decisions Q3). KF-16 (enable write inside iteration) and KF-18 (dense x enable iteration) are no longer needed for physics: main:docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md:504 "**Not needed:** an in-place `EnableMut` toggle." Gameplay sees sleep and wake as kernel events, O(transitions). This also removes latent defect A (the latch keyed by gather row) by construction: a slot never moves and a reused slot starts DEAD = awake (D14).

**SoftBody** (rows: rows 9-70)

- *Entity model:* The soft body is an entity; its particles are NOT, because per-body contiguity is the solver's contract and LIFO dense-slot reuse cannot keep it (joltab:crates/boyko_ecs/src/ecs/core/component/dense/dense_store.rs:124 "/// LIFO free list of tombstoned slots. `insert` pops here first so a freed"). [rev2 GAP 3]
- *ECS form:* dense-component in the K7 segmented dense column (particle state AND topology, per body contiguous) + shared system-scratch for the per-substep fields (main:docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md:314 "\| `SoftBody` per-substep scratch (10) \| shared system scratch \| — \| Bodies are stepped serially (`soft/colored.rs:699`) \|")
- *Hot-loop cost:* Identical strides - the kernels already take raw column bases once per dispatch (boyko_physics/src/soft/solver.rs:490 "pos_x: body.pos_x.as_mut_ptr(),"). Gain: one dispatch per colour across bodies instead of per colour per body (D:/claude/BoykoEngine/docs/physics/ADVANCED-PHYSICS-DESIGN-SPACE.md:853 "per-substep `pool.scope` COUNT goes from "one per dispatching colour per body" to "one per"), with the small-body regime caveat D-8 records (C-NB3).
- *Missing kernel feature:* physics K7 (segmented dense column) on K3 dense groups; KF-03 owned span is its ragged-range half.

**Solver ScratchColumn cohorts (scratch_ids.rs)** (rows: none (already ScratchColumn))

- *Entity model:* Per-step solver working set: contact slots in colour order, body mirrors, graph CSR. Not entities.
- *ECS form:* system-scratch (resource-owned ScratchColumns), already done by Stage 4.
- *Hot-loop cost:* Stage 4 measured no end-to-end cost; the id bands exist only because the id doubles as the cache-set stagger key (boyko_ecs/src/ecs/constants.rs:214 "pub const fn pool_base_stagger(component_id: usize) -> usize {").
- *Missing kernel feature:* KF-column-id-per-type-with-explicit-stagger would replace the hand-maintained bands and the floor coupling (boyko_physics/src/scratch_ids.rs:541 "const SCRATCH_REGION_MIN_ID: usize = MAX_COMPONENTS - 128;").

**ContactPairs / Manifolds (is a contact pair an entity?)** (rows: none (already ScratchColumn))

- *Entity model:* No. A pair-entity would cost structural churn per contact begin/end at the house spawn rate (boyko_render/src/particle.rs:5 "20k spawns/frame through `Commands` is 0.6–2 ms of CPU"), its archetype-row order would depend on spawn/despawn history (D:/wt/joltab/docs/ARCH-AUDIT-ECS-DATA-REMEDIATION.md:30 "NOT pair-entities — W2: pair-entity archetype-row order depends on spawn/despawn/recycle order"), and the solver still needs colour-ordered contiguous SoA lanes, so a per-step re-gather would remain. The gameplay-facing projection is separate: the Contact component exists with no producer, and begin/end events are the missing piece (D:/claude/BoykoEngine/docs/physics/ADVANCED-PHYSICS-DESIGN-SPACE.md:1282 "**Contact begin/end and sensor EVENTS, plus the `Contact` producer**").
- *ECS form:* system-scratch stage streams (resource-owned ScratchColumns) for the solver; component Contact + event (contact begin/end) for gameplay - the owner's open call (D:/wt/joltab/docs/ARCH-AUDIT-ECS-DATA-REMEDIATION.md:42 "**Contacts as gameplay-visible ECS data**").
- *Hot-loop cost:* Unchanged; events are emitted once per step after the solve from the canonical-order data.
- *Missing kernel feature:* KF-lossless-hook-event's growable lanes (contact events tolerate next-frame visibility but not silent loss); KR-1 to project rows to entities.
- *Rev 2:* [rev2] The open call is decided by physics Q1: kernel events only (contact and sensor enter/exit). main:docs/physics/PHYSICS-ECS-UNIFICATION-DECISIONS.md:29 "\| Q1 \| How gameplay sees contacts \| **(a) Kernel events only**: contact and sensor enter/exit \|"

**ConstraintGraph / BroadphaseGrid** (rows: row 8 (debug re-scan scratch) only)

- *Entity model:* Islands, colours and grid cells are per-step partitions, not entities (island ids volatile; up to 2^21 cells).
- *ECS form:* system-scratch.
- *Hot-loop cost:* Unchanged. The residual against Jolt is serial stages, not storage: broadphase serial below 4096 bodies, narrowphase a single loop, colours under 256 slots inline.
- *Missing kernel feature:* KF-in-scope-barrier + ordered-parallel-emit (KE16 lane).

**RefcountDeltas / DeferredFree / TransformPropagationScratch (scene)** (rows: rows 71-73, 78-81)

- *Entity model:* RefDelta and detach are transition messages (events); FreeEntry is a durable fence-gated queue of asset rows (resource-column); stack/dirty are one run's traversal state (system-scratch).
- *ECS form:* event / resource-column / system-scratch.
- *Hot-loop cost:* Negligible (churn-small queues); DeferredFree's drain goes from O(n^2) to O(n).
- *Missing kernel feature:* KF-lossless-hook-event; KF-column-id-per-type-with-explicit-stagger (naming: a kernel per-type mint rather than the asset module's).

**PairCache (WarmStartTable + BoxAxisCache) [rev2 GAP 2]** (rows: solver/warm_start.rs:214, narrowphase/axis_cache.rs:128 (rev2 rows))

- *Entity model:* Not an entity: a contact pair has two owners and pair entities are rejected (main:docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md:159 "### D5: Per-pair persistent state is one resource-owned, double-buffered `PairCache`, keyed by slots (preserved)"). The datum is per-pair and DURABLE across one step boundary.
- *ECS form:* resource-column PairCache keyed by the stable dense-group body slot, double-buffered, fresh_step skip (main:docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md:268 "- Every `PairCache` lookup (S3 axis read, S5 warm seed) is skipped when `fresh_step[a] == step \|\| fresh_step[b] == step`.").
- *Hot-loop cost:* Equal: same key width, same probe; the read-old/write-new split lets narrowphase run parallel (joltab:crates/boyko_physics/src/systems.rs:422 "axis_cache.set(a, b, c.reference_axis);").
- *Missing kernel feature:* The stable body slot (physics D1 / K3 dense group) - without it the key stays the gather row (DC-PAIR-1).

## Kernel features

The groups and lanes proposed 77 kernel features between them. Deduplicated, they form **49 features**: the 43 of rev 1, KF-44 to KF-46 from the gaps, KF-47 from the reflect lane (the ui-lane needed no new one), and KF-48 / KF-49 from rev 3 (engine Q1). Each is a first-class `boyko_ecs` capability (or a memory-library / pool capability below it) that every crate uses the same way. **Status** says what rev 2 did with it: `active`, `decided` (an open fork closed in section Decisions), `withdrawn` (no user left), `rejected` (a design decision replaced it), `subsumed` / `superseded` (another feature covers it). **K** names the physics design's kernel feature that realises it, **EK** the engine design's. **Rows** are active rows whose form needs the feature.

| id | feature | status | physics | K | EK | rows | rows by group |
|---|---|---|---|---|---|---|---|
| KF-01 | ScratchColumn by type (kernel-minted id, explicit stagger) | decided | yes | K1 | EK1 | 324 | app-demo 7, ecs-schedule 44, ecs-services 7, ecs-storage 35, physics-scene-math 85, render 3, rhi 31, ui-input 22, ui-lane 90 |
| KF-02 | Owning scratch column (non-Copy elements with drop glue) | active | no | - | EK12 | 9 | ecs-services 1, ecs-storage 4, pool-utils-log 1, render 2, rhi 1 |
| KF-03 | Owned span / ragged ranges on kernel columns | active | yes | K7 | EK14 | 40 | ecs-storage 4, physics-scene-math 36 |
| KF-04 | Durable resource column (lifetime + serialization) | active | yes | - | - | 10 | physics-scene-math 6, ui-lane 4 |
| KF-05 | World scratch frames (re-entrant LIFO scratch for &mut paths) | decided | no | - | - | 15 | ecs-storage 15 |
| KF-06 | Byte column (bulk append + fmt::Write) | active | no | - | - | 13 | ecs-services 6, ui-input 5, ui-lane 2 |
| KF-07 | Erased record column (heterogeneous drop-aware records) | active | no | - | - | 6 | ecs-schedule 6 |
| KF-08 | Serialize seam on kernel columns | active | no | - | - | 9 | codec-tools 8, ecs-services 1 |
| KF-09 | Loader decode context | active | no | - | - | 65 | codec-tools 18, ecs-services 3, render 44 |
| KF-10 | Structured asset error | active | no | - | - | 77 | ecs-services 3, render 74 |
| KF-11 | Relation reverse index on kernel storage (K7 spans) | decided | yes | K7 | EK15c | 4 | ecs-services 3, ecs-storage 1 |
| KF-12 | Multi-target relation (OPTIONAL) | active | no | - | - | 18 | ecs-schedule 18 |
| KF-13 | Default-excluded (hidden) entities; prefab templates as entities | decided | no | - | EK18 | 8 | ecs-storage 8 |
| KF-14 | System entities | decided | no | - | - | 26 | ecs-schedule 26 |
| KF-15 | Observer entities | decided | no | - | - | 5 | ecs-storage 5 |
| KF-16 | Enable write inside iteration | withdrawn | no | - | - | 0 (2 named it) | physics-scene-math 2 |
| KF-17 | Enable initial polarity | active | no | - | EK4 | 8 | render 8 |
| KF-18 | Dense x enable iteration (existing plan) | withdrawn | no | - | - | 0 (2 named it) | physics-scene-math 2 |
| KF-19 | Dense slot access (typed, scheduler-visible) | active | yes | K3 | - | 14 | app-demo 11, render 3 |
| KF-20 | Dense par_iter / par_for_each_chunk | active | yes | K4 | - | 6 | app-demo 6 |
| KF-21 | In-scope barrier + ordered parallel emit | active | yes | K5a, K5b | - | 0 | - |
| KF-22 | Entity datum in Query (KR-1) | subsumed | yes | K3 | - | 0 (0 named it) | - |
| KF-23 | Event lane policies (lossless / drop-oldest / coalesce / non-system producers) | active | yes | - | EK7 (in part) | 5 | physics-scene-math 2, rhi 1, ui-input 2 |
| KF-24 | Same-frame event delivery | rejected | no | - | - | 0 (4 named it) | physics-scene-math 2, ui-input 1, ui-lane 1 |
| KF-25 | Change-detection query surface | active | no | - | - | 3 | physics-scene-math 2, ui-lane 1 |
| KF-26 | Kernel name table | active | no | - | EK20 | 4 | physics-scene-math 4 |
| KF-27 | Query into kernel column | active | no | - | - | 12 | ui-lane 12 |
| KF-28 | World read beside Commands | active | no | - | - | 6 | ui-lane 6 |
| KF-29 | Startup schedule | active | no | - | EK9 | 2 | ecs-services 2 |
| KF-30 | App runner as fn pointer | active | no | - | - | 1 | app-demo 1 |
| KF-31 | App owns the pool; PoolInner in one reservation | active | no | - | - | 19 | app-demo 3, pool-utils-log 16 |
| KF-32 | Memory library below the pool | active | no | - | - | 9 | pool-utils-log 9 |
| KF-33 | Scope arena on the memory library | active | no | - | - | 4 | pool-utils-log 4 |
| KF-34 | In-house work-stealing lanes | active | no | - | - | 4 | pool-utils-log 4 |
| KF-35 | VmColumn ensure_len_zeroed | active | no | - | - | 1 | pool-utils-log 1 |
| KF-36 | Kernel storage reachable from the RHI (+ generational table) | decided | no | - | - | 44 | pool-utils-log 3, rhi 41 |
| KF-37 | Assets&lt;T>::adopt_retiring | superseded | no | - | K6' (engine) | 0 (2 named it) | render 2 |
| KF-38 | Assets&lt;T>::iter_mut + owned drain | withdrawn | no | - | - | 0 (3 named it) | render 3 |
| KF-39 | ComponentPool add inert row | active | no | - | - | 1 | ecs-services 1 |
| KF-40 | Device column residency (existing seam) | active | no | - | EK16 | 2 | render 2 |
| KF-41 | Intrusive free lists | decided | no | - | - | 4 | ecs-storage 4 |
| KF-42 | Atomic views over kernel columns | active | no | - | - | 4 | ecs-storage 4 |
| KF-43 | Static type descriptors (derive output allocation-free) | active | no | - | - | 5 | macros-aether 5 |
| KF-44 | Per-system state for exclusive systems | active | no | - | EK2 | 21 | physics-scene-math 4, ui-lane 17 |
| KF-45 | Engine thread-context column (replaces every thread_local!) | active | yes | - | - | 12 | ecs-services 3, ecs-storage 3, pool-utils-log 6 |
| KF-46 | Window entities | active | no | - | ED18 (engine) | 13 | rhi 11, ui-input 2 |
| KF-47 | By-id structural seam (structural ops by ComponentId + bytes) | active | no | - | - | 2 | reflect-lane 2 |
| KF-48 | Count-only relation (asset refcount) | active | no | - | EK15b | 1 | physics-scene-math 1 |
| KF-49 | Deferred dense-group release with a horizon (K6') | active | conditional | K6 | K6' (engine) | 4 | physics-scene-math 1, render 3 |

Distinct active rows that need at least one kernel feature: **657**.

### The physics design's K1-K7, cross-referenced

The physics unification design orders its own kernel features K1-K7 (design section 8). Each is listed with the ledger features it realises; its row text is quoted from the design.

| K | feature | design row | ledger features |
|---|---|---|---|
| K1 | Storage cohorts | main:docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md:466 "\| K1 \| Storage cohorts \| `ScratchCohort::reserve(width)`: registry-free scratch pools, contiguous stagger run. `DenseGroup` registration mints column" | KF-01 |
| K2 | Untracked dense, compile-time sound | main:docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md:467 "\| K2 \| Untracked dense, compile-time sound \| See the table below \| All group columns \|" | (no rev-1 ledger feature; physics group columns carry no ticks) |
| K3 | Dense groups | main:docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md:468 "\| K3 \| Dense groups \| See the list below \| `PhysicsBody` \| render instance families (candidate; not verified); animation skeleton instances \| K1, K2 \|" | KF-19, KF-22 (subsumed); the BodyGate rows (W3) |
| K4 | KE15: par_iter over mixed table + dense terms | main:docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md:469 "\| K4 \| KE15: `par_iter` / `par_for_each_chunk` over mixed table + dense / `GroupSlot` terms \|" | KF-20 |
| K5a | par_range | main:docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md:470 "\| K5a \| `par_range` \| A one-phase gang with a caller-supplied cut table \| S2 AllPairs \| render culling/batching, scene propagation \| K5b \|" | KF-21 |
| K5b | Gang (par_phases) + task run context | main:docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md:471 "\| K5b \| Gang (`par_phases`) + task run context \| §10.1, including the nesting rule \| S2 grid, S3, S5, soft solve \|" | KF-21 |
| K6 | Group release command | main:docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md:472 "\| K6 \| Group release command \| `Commands::release_dense_group::<G>()` and `EcsMaster::release_dense_group::<G>()` \| S6 \| any deferred-release group \|" | (no rev-1 ledger feature; the engine's K6' generalises it and supersedes KF-37) |
| K7 | Segmented dense column | main:docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md:473 "\| K7 \| Segmented dense column \| Per-entity variable-length ranges in a group bank \| SoftBody (Q2) \| animation bone arrays, UI text runs \| K3 \|" | KF-03 mode (a), KF-11 backing (EK15c); the SoftBody rows (gap 3) |

### KF-01 ScratchColumn by type (kernel-minted id, explicit stagger)

- **Status:** decided. **Physics needs it:** yes. **Kind:** capability. **Physics design:** K1. **Engine design:** EK1.
- **Rev 2:** Minting route decided (section Decisions): the physics design's K1 storage cohorts, which the engine design adopts as EK1: a registry-free scratch band that does not consume component ids, with a contiguous stagger run per cohort. The ui-lane reached the same answer (UL-D6). main:docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md:466 "\| K1 \| Storage cohorts \| `ScratchCohort::reserve(width)`: registry-free scratch pools, contiguous stagger run. `DenseGroup` registration mints column" ; main:docs/unification/ENGINE-RUNTIME-ECS-DESIGN.md:893 "\| EK1 **(rev 2)** \| **= physics K1 storage cohorts**, plus `impl Default for ScratchColumn<T>` (a width-1".
- **Rev 3:** Rev 3: users added by the ui-lane census outside boyko_ui (item 1) and the numeric diagnostics captures re-formed to system-scratch (item 4).
- **Adds to the kernel:** Any crate can build a `ScratchColumn<T: Copy>` from the element TYPE alone (plus `Default`, so a system can hold `Local<ScratchColumn<T>>`). The layout id is minted by ONE kernel registry, keyed by type or `Layout`, from a kernel-managed scratch band. The cache-set stagger is passed explicitly instead of being derived from a distinct id. Today the constructor needs a caller-minted, pre-registered `ComponentId`. As a result, physics hand-keeps id bands (`scratch_ids.rs`), render borrows the asset layout registry, and UI and input have no route at all: three crate-local answers to one kernel need.
- **Crates:** boyko_ecs (provides); boyko_physics, boyko_ui, boyko_input, boyko_render, boyko_rhi_vulkan, boyko_demo, boyko_scene, boyko_sdf_math callers
- **Plan:** Planned and never shipped: ARCH-AUDIT-ECS-DATA-REMEDIATION Stage 0 `ComponentPool::new_scratch` (quoted in evidence). The minting route was open in rev 1: ecs-storage, codec-tools and app-demo reuse `register_asset_layout`. physics records that this climbs the production id counter. rhi wants it registry-free. Rev 2 decides it (the Rev 2 line above).
- **Merged from:** ecs-storage `KF-scratch-column-for-type`; ecs-schedule `KF-typed-scratch-id`; physics-scene-math `KF-column-id-per-type-with-explicit-stagger`; rhi `KF-registry-free-scratch-column`; ui-input `KF1-scratch-column-for-type`; ui-lane `KF-01 ScratchColumn by type (kernel-minted id, explicit stagger)`; reflect-lane `ScratchColumn by type (kernel-minted id, explicit stagger)`
- **Rows (324):** crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs:328,342,354,367,383,398,418,435,449,462; crates/boyko_ecs/src/ecs/core/archetype/archetype_registry.rs:126,180,199,226,317,335,396,419; crates/boyko_ecs/src/ecs/core/clone/deep.rs:88,94,137,246; crates/boyko_ecs/src/ecs/core/clone/map.rs:22; crates/boyko_ecs/src/ecs/core/clone/materialize.rs:861; crates/boyko_ecs/src/ecs/core/component/dense/dense_store.rs:794; crates/boyko_ecs/src/ecs/core/ecs_master/component_api.rs:795,838; crates/boyko_ecs/src/ecs/core/ecs_master/entity_query_api.rs:92,111,112,135,136; crates/boyko_ecs/src/ecs/core/ecs_master/observer_api.rs:636,637,681; crates/boyko_ecs/src/ecs/core/iters/query/relation/traverse_iter.rs:51,284,300,314; crates/boyko_ecs/src/ecs/core/schedule/conflict_graph.rs:122; crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs:485,523,578,589,591,612,614,633,634,906,907,908,951,953,960,961,962,965,966,972,1013,1048,1049,1050,1057,1064,1193,1198,1215,1216,1230,1235,1266,1327,1332; crates/boyko_ecs/src/ecs/core/system/dispatcher_token.rs:352,353; crates/boyko_ecs/src/ecs/core/system/filtered_access_set.rs:127,146; crates/boyko_ecs/src/ecs/core/profiling/analysis.rs:150,198,199,200,201,203,292; crates/boyko_physics/src/resources.rs:2965,3068,3073,3106,3107; crates/boyko_physics/src/soft/component.rs:71,73,75,77,79,81,83,85,87,89,91,93,95,98,105,107,109,111,115,118,129,131,133,142,144,146,154,163,171,177,327,328,329,330,331,374,499,500,501,508,509,510,511,512,513,514,516,517,524,526,540,541,569,570,571,572,573,574,575,580,581,582; crates/boyko_scene/src/asset_refs.rs:99,149,180; crates/boyko_scene/src/propagation.rs:120,124,133,404; crates/boyko_sdf_math/src/mesh_sdf.rs:298,301,316,319,320,321,341,357,791,800,814; crates/boyko_render/src/vg_census.rs:114,134,137; crates/boyko_rhi/src/handle.rs:78; crates/boyko_rhi_vulkan/src/device.rs:2358,2413,2545,2721,3114; crates/boyko_rhi_vulkan/src/framegraph/graph.rs:126,127,136,145,148,156,157,158,161,162,163,164,165,172,188,200,203,204,205; crates/boyko_rhi_vulkan/src/memory.rs:700; crates/boyko_rhi_vulkan/src/present/surface.rs:180,238; crates/boyko_rhi_vulkan/src/suballocator.rs:65,68; crates/boyko_rhi_vulkan/src/window.rs:761; crates/boyko_input/src/action/map.rs:134,136,201,202,242,275,283,330,332,368,369; crates/boyko_input/src/persist/grammar.rs:386,387,398,471,473; crates/boyko_input/src/persist/keyname.rs:215,249; crates/boyko_input/src/persist/writer.rs:30,57,58; crates/boyko_input/src/plugin.rs:100; crates/boyko_app/src/gpu_scene/particle.rs:1083; crates/boyko_app/src/host_dump.rs:170,210; crates/boyko_app/src/profiling/artifact.rs:854,856; crates/boyko_app/src/profiling/contrast.rs:136; crates/boyko_app/src/profiling/reduce.rs:635; [ui-lane] crates/boyko_render/src/ui/gather.rs:284,289; [ui-lane] crates/boyko_render/src/ui/pack.rs:833; [ui-lane] crates/boyko_render/src/ui/upload.rs:198,206,669; [ui-lane] crates/boyko_ui/src/animation.rs:473; [ui-lane] crates/boyko_ui/src/binding/bind_system.rs:46,48; [ui-lane] crates/boyko_ui/src/interaction/focus.rs:117,119,122,128,131,136; [ui-lane] crates/boyko_ui/src/layout.rs:215; [ui-lane] crates/boyko_ui/src/plugin.rs:93; [ui-lane] crates/boyko_ui/src/reload/reconcile.rs:77,116,131,132,140,189,190,248,249,276,334,665; [ui-lane] crates/boyko_ui/src/reload/state.rs:83; [ui-lane] crates/boyko_ui/src/reload/system.rs:92,110; [ui-lane] crates/boyko_ui/src/reload/tree_view.rs:91; [ui-lane] crates/boyko_ui/src/resources.rs:216,218,226,234,239,247,254,323,324,325,327,328,329,335,336,337,338; [ui-lane] crates/boyko_ui/src/sprite.rs:257; [ui-lane] crates/boyko_ui/src/text/ast.rs:31,34,73,75,112,114; [ui-lane] crates/boyko_ui/src/text/emit.rs:65,89,141; [ui-lane] crates/boyko_ui/src/text/font.rs:33,36,38,41,52,55,72,75,139; [ui-lane] crates/boyko_ui/src/text/lower.rs:67,69,83; [ui-lane] crates/boyko_ui/src/text/parser.rs:48,49,51,61,281,310,311,319,320,334; [ui-lane] crates/boyko_ui/src/text/serialize.rs:32,39; [ui-lane] crates/boyko_ui/src/text/split.rs:59,61; [ui-lane] crates/boyko_ui/src/widgets.rs:73; [ui-lane] crates/boyko_ui/src/world/pick.rs:110,113,117
- **Evidence:**
  - crates/boyko_ecs/src/ecs/core/component/scratch/scratch_column.rs:88 "pub fn new(component_id: ComponentId, reserve_rows: usize) -> Self {"
  - crates/boyko_ecs/src/ecs/core/asset/backing.rs:115 "pub fn register_asset_layout&lt;T: 'static>(drop_fn: Option&lt;DropFn>) -> ComponentId {"
  - crates/boyko_physics/src/scratch_ids.rs:17 "the scratch ids occupy a fixed region at"
  - crates/boyko_ecs/src/ecs/core/system/params/local.rs:62 "pub struct Local&lt;'s, T: Send + Sync + Default + 'static>(pub(crate) &'s mut T);"
  - crates/boyko_ecs/src/ecs/memory/component_pool.rs:2384 "unsafe impl Send for ComponentPool {}"
  - crates/boyko_ecs/src/ecs/core/component/scratch/scratch_column.rs:56 "The caller owns the id assignment"
  - crates/boyko_ecs/src/ecs/core/component/component_registry/mod.rs:1031 "pub fn register_layout&lt;T: 'static>(component_id: usize) {"
  - crates/boyko_ecs/src/ecs/core/component/component_registry/mod.rs:1020 "non-`Component` element type"
  - crates/boyko_physics/src/scratch_ids.rs:834 "ComponentId::new(SCRATCH_ID_BODY_EFF_SERIAL)"
  - crates/boyko_ecs/src/ecs/core/component/component_registry/mod.rs:63 "pub const MAX_COMPONENTS: usize = 512;"
  - boyko_ecs/src/ecs/core/asset/backing.rs:115 "pub fn register_asset_layout&lt;T: 'static>(drop_fn: Option&lt;DropFn>) -> ComponentId {"
  - boyko_render/src/mesh_draw.rs:429 "let u32_id = register_asset_layout::&lt;u32>(None);"
  - boyko_render/src/mesh_draw.rs:446 "counts: ScratchColumn::new(u32_id, u32_rows),"
  - boyko_render/src/mesh_draw.rs:447 "offsets: ScratchColumn::new(u32_id, u32_rows),"
  - boyko_ecs/src/ecs/constants.rs:214 "pub const fn pool_base_stagger(component_id: usize) -> usize {"
  - boyko_physics/src/scratch_ids.rs:166 "at the same moment, so distinct cohorts may reuse the same slots freely."
  - boyko_physics/src/scratch_ids.rs:538 "At 128 the scratch side keeps ~38 ids of headroom"
  - boyko_ecs/src/ecs/core/component/component_registry/mod.rs:968 "let mut current = NEXT_ID.load(Ordering::Relaxed);"
  - Planned and not shipped: docs/ARCH-AUDIT-ECS-DATA-REMEDIATION.md:27 "`ComponentPool::new_scratch(layout, reserve_rows)` (synthetic-id, registry-free, tick sub-regions reserved-uncommitted)". Shipped instead: boyko_ecs/src/ecs/core/component/scratch/scratch_column.rs:88 "pub fn new(component_id: ComponentId, reserve_rows: usize) -> Self {", which needs an id whose layout is registered in the process-global table (boyko_ecs/src/ecs/core/component/component_registry/mod.rs:208 "static LAYOUTS: [OnceLock&lt;ComponentLayout>; MAX_COMPONENTS] =") capped at boyko_ecs/src/ecs/core/component/component_registry/mod.rs:63 "pub const MAX_COMPONENTS: usize = 512;"; register_new mints a FRESH id per call (boyko_ecs/src/ecs/core/component/component_registry/mod.rs:920 "pub fn register_new&lt;T: 'static>() -> usize {"; boyko_ecs/src/ecs/core/component/component_registry/mod.rs:921 "let raw = NEXT_ID.fetch_add(1, Ordering::Relaxed);"), and the untracked constructor is pub(crate) (boyko_ecs/src/ecs/memory/component_pool.rs:501 "pub(crate) fn new_untracked(component_id: usize, reserve_rows: usize) -> Self {"). Physics works around it with a hand-managed band, one id per column (boyko_physics/src/scratch_ids.rs:557 "ComponentId::new(SCRATCH_ID_BROADPHASE_TOP - k)"; boyko_physics/src/scratch_ids.rs:541 "const SCRATCH_REGION_MIN_ID: usize = MAX_COMPONENTS - 128;"; boyko_physics/src/scratch_ids.rs:537 "/// Finishing Stage 4 needs roughly 90 scratch ids, which does not fit under 64."). The FrameGraph alone would take 17 (release) to 19 (debug) more ids. Not strictly blocking - ids could be minted from the band today - but every crate hand-managing ids is the crate-local workaround the owner's rule forbids.
  - boyko_ecs/src/ecs/core/component/scratch/scratch_column.rs:88 "pub fn new(component_id: ComponentId, reserve_rows: usize) -> Self {"
  - boyko_physics/src/scratch_ids.rs:555 "pub(crate) fn broadphase_column_id(k: usize) -> ComponentId {"
  - D:/wt/ui:crates/boyko_ecs/src/ecs/core/component/scratch/scratch_column.rs:62 "pub fn new(component_id: ComponentId, reserve_rows: usize) -> Self {"
  - D:/wt/joltab:crates/boyko_physics/src/scratch_ids.rs:555 "pub(crate) fn broadphase_column_id(k: usize) -> ComponentId {"
  - D:/wt/ui:crates/boyko_render/src/mesh_draw.rs:429 "let u32_id = register_asset_layout::&lt;u32>(None);"
  - D:/wt/reflect:docs/REFLECTION-PLAN-BOUNDARY.md:602 "\| resolution shape \| `resolve_type_table` → dense `Vec<ResolvedType>` by file-local index"
- **Group notes:**
  - (ecs-schedule) The band lives inside MAX_COMPONENTS = 512, so keying by Layout (size, align) rather than by type keeps it finite; Copy-only elements need no drop glue, so the layout is all the pool needs.
  - (physics-scene-math) Caveat on the per-type mint: it climbs the PRODUCTION id counter (boyko_ecs/src/ecs/core/component/component_registry/mod.rs:968 "let mut current = NEXT_ID.load(Ordering::Relaxed);"), so every distinct scratch element type counts against the 384-id production margin the physics floor protects (boyko_physics/src/scratch_ids.rs:541 "const SCRATCH_REGION_MIN_ID: usize = MAX_COMPONENTS - 128;"). Sharing an id without an explicit stagger also shares a cache set - harmless for a lone queue column, the P2 conflict-miss storm for a swept cohort.
  - (ui-input) Replaces the verified file's new_primitive "public, ComponentId-free VmColumn&lt;T: Copy> / ScratchColumn constructor"; exporting VmColumn is refuted (R6). The UNTRACKED backing (no change ticks) is right for scratch; resource-column users (InputMap, fonts) rely on Resource-level change detection (ResMut), which they already do.
  - (ui-lane, note) Replaces the verified file's new_primitive "public, ComponentId-free VmColumn&lt;T: Copy> / ScratchColumn constructor"; exporting VmColumn is refuted (R6). The UNTRACKED backing (no change ticks) is right for scratch; resource-column users (InputMap, fonts) rely on Resource-level change detection (ResMut), which they already do. \| ui-lane: +1 user (UiSheetTable::sheets, sprite.rs:257) and the fallback of UiTweenScratch::done. DECIDED 2026-09-11 (section (i) 'Scratch id minting route', 'One kernel decision'): a kernel scratch band keyed by Layout (size, align) with the cache-set stagger passed explicitly. Performance: it keeps scratch ids off the production counter (the 384-id margin the physics floor protects), a per-Layout key bounds the band (the UI lanes use about 20 distinct element layouts), and an explicit stagger costs nothing for a lone UI lane while it is what prevents the P2 conflict-miss storm for a swept physics cohort. Overturned by a band-exhaustion count, or by a measured conflict-miss rate on a UI lane that a per-type id would have avoided.
  - (ui-lane, capability) ScratchColumn&lt;T: Copy> constructible from any crate by TYPE alone (ScratchColumn::&lt;T>::for_type(reserve_rows)), with the layout id minted by ONE kernel TypeId->layout registry. Today the constructor needs a caller-minted, pre-registered ComponentId, so physics hand-rolls id cohorts, render borrows the ASSET layout registry for u32, and UI/input have no route at all - three crate-local answers to one kernel need.
  - (reflect-lane, note) The BOUNDARY plan adopts boyko_serialize's dense Vec&lt;ResolvedType> deliberately; decided: system-scratch on a ScratchColumn&lt;ResolvedType> exactly like the ledger's boyko_serialize load.rs rows. Needs KF-01 to mint the column id from the type.
  - (reflect-lane, status) existing ledger feature - NEEDED by PLANNED BOUNDARY rung B3, no row today

### KF-02 Owning scratch column (non-Copy elements with drop glue)

- **Status:** active. **Physics needs it:** no. **Kind:** capability. **Engine design:** EK12.
- **Rev 2:** The engine design's EK12 (main:docs/unification/ENGINE-RUNTIME-ECS-DESIGN.md:904 "\| EK12 \| Owning scratch column \| non-`Copy` column with drop glue \| allocator `DropColumn` \|").
- **Adds to the kernel:** A typed column over an untracked `ComponentPool` for non-Copy `T` with registered drop glue. It offers push, typed move-out (`take_at`), `retain_ready(epoch)` / drain-by-predicate, and a clear that honours the drop glue. `Assets<T>` already implements this view privately. It is also the single uniform fence-retire lane: six retire-lane types share the (value, retire_frame) shape.
- **Crates:** boyko_ecs (provides); boyko_render, boyko_rhi_vulkan, boyko_app; pool-utils-log only conditionally
- **Plan:** No plan owns it. It conflicts with ALLOCATOR-DESIGN-SPACE `DropColumn<T>` (the same capability, but as a new primitive on VmReservation).
- **Merged from:** render `KF-owning-scratch-column`; ecs-storage `KF-owning-scratch-column`; ecs-services `KF-owning-scratch-column`; rhi `KF-owning-scratch-column`; pool-utils-log `KF-owning-scratch-column (render group's name; CONDITIONAL here)`
- **Rows (9):** crates/boyko_ecs/src/ecs/core/component/component_pool_bundle.rs:13; crates/boyko_ecs/src/ecs/core/component/dense/dense_registry.rs:78; crates/boyko_ecs/src/ecs/core/component/enable/enable_store.rs:769; crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs:953; crates/boyko_ecs/src/ecs/core/asset/staging.rs:58; crates/boyko_utils/src/sparse_map/sparse_map.rs:10; crates/boyko_render/src/retired_gpu_buffers.rs:53,59; crates/boyko_rhi_vulkan/src/memory.rs:700
- **Evidence:**
  - boyko_ecs/src/ecs/core/component/scratch/scratch_column.rs:43 "pub struct ScratchColumn&lt;T: Copy> {"
  - boyko_ecs/src/ecs/memory/component_pool.rs:1694 "pub(crate) unsafe fn take_at&lt;T: 'static>(&mut self, idx: usize) -> T {"
  - boyko_ecs/src/ecs/core/asset/backing.rs:115 "pub fn register_asset_layout&lt;T: 'static>(drop_fn: Option&lt;DropFn>) -> ComponentId {"
  - crates/boyko_ecs/src/ecs/memory/component_pool.rs:279 "pub fn new(component_id: usize, reserve_rows: usize) -> Self {"
  - crates/boyko_ecs/src/ecs/core/asset/backing.rs:115 "pub fn register_asset_layout&lt;T: 'static>(drop_fn: Option&lt;DropFn>) -> ComponentId {"
  - crates/boyko_render/src/mesh.rs:231 "register_asset_layout::&lt;MeshGpu>(Some(MeshGpu::drop_glue))"
  - crates/boyko_ecs/src/ecs/memory/component_pool.rs:1694 "pub(crate) unsafe fn take_at&lt;T: 'static>(&mut self, idx: usize) -> T {"
  - crates/boyko_ecs/src/ecs/memory/vm_column.rs:30 "is deliberately NOT a general `Vec` replacement for droppable `T`."
  - crates/boyko_ecs/src/ecs/core/component/scratch/scratch_column.rs:43 "pub struct ScratchColumn&lt;T: Copy> {"
  - ScratchColumn rejects drop types (boyko_ecs/src/ecs/core/component/scratch/scratch_column.rs:91 ""ScratchColumn requires a POD (Copy, no Drop) element type; \"); the pool itself already runs drop glue (boyko_ecs/src/ecs/memory/component_pool.rs:237 "drop_fn: Option&lt;DropFn>,"; boyko_ecs/src/ecs/memory/component_pool.rs:1107 "/// Removes the last component from the pool, invoking drop glue if needed.") and registers any T (boyko_ecs/src/ecs/core/component/component_registry/mod.rs:1031 "pub fn register_layout&lt;T: 'static>(component_id: usize) {"), but its typed move-in needs T: Component (boyko_ecs/src/ecs/memory/component_pool.rs:897 "pub fn add_typed&lt;T: Component>(&mut self, value: T) -> Option&lt;usize> {").
  - crates/boyko_ecs/src/ecs/memory/component_pool.rs:237 "drop_fn: Option&lt;DropFn>," (the pool already carries drop glue)
- **Group notes:**
  - (render) It is also the uniform fence-retire lane. Read this session: six retire-lane types carry a (value, retire_frame) shape - DeferredFree.entries (boyko_scene), OrphanedMeshGpu, OrphanedTextureGpu, RetiredGpuBuffers.entries, RetiredGpuBuffers.tlases, and BindlessSlotAllocator.retiring_slots (instantiated by two tables). One column type with one retain_ready(epoch) drain serves all six, and a single scheduled retire system can drain them. Other groups' non-Copy Resource tables probably need it as well; not counted. PLAN CONFLICT: D:/claude/BoykoEngine/docs/memory/ALLOCATOR-DESIGN-SPACE.md:111 "`ScratchColumn` + new `DropColumn<T>`" proposes DropColumn&lt;T> as a new column primitive on VmReservation; this is the same capability as a typed view over the existing ComponentPool.
  - (ecs-services) Deduplicate with render.ecsform.json; this group adds one user.
  - (pool-utils-log) Not needed if block_groups is flattened to Copy ranges, which is the recommendation.

### KF-03 Owned span / ragged ranges on kernel columns

- **Status:** active. **Physics needs it:** yes. **Kind:** capability. **Physics design:** K7. **Engine design:** EK14.
- **Rev 2:** Mode (a) is the physics design's K7 segmented dense column, which now holds the SoftBody particle state and topology (gap 3, physics Q2) and which the engine design adopts as EK14; mode (b), write-once CSR, runs on two K1 columns with no new feature (engine design line 437). main:docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md:473 "\| K7 \| Segmented dense column \| Per-entity variable-length ranges in a group bank \| SoftBody (Q2) \| animation bone arrays, UI text runs \|".
- **Adds to the kernel:** Variable-length per-owner POD lists stored on kernel columns as a flat element column plus `(start, len)`. There are two modes. (a) Owned span: a handle component `{start, n}` names a span in resource- or asset-owned columns; the span is allocated by the handle's insert hook, returned by its remove hook, the range free list lives on kernel columns, and assignment is a pure function of the op sequence. (b) CSR, rebuilt on mutate, for register-time tables. Physics already hand-builds a CSR (BroadphaseGrid).
- **Crates:** boyko_ecs (provides); boyko_physics (SoftBank, SoftAsset), boyko_ui, the observer/trigger tables; candidates not examined: animation pose bank, fracture chunks
- **Plan:** ADVANCED-PHYSICS-DESIGN-SPACE D-8 owns mode (a) (quoted in evidence).
- **Merged from:** physics-scene-math `KF-owned-span`; ecs-storage `KF-ragged-column`; reflect-lane `Kernel name table / owned span on kernel columns`
- **Rows (40):** crates/boyko_ecs/src/ecs/core/component/observers/mod.rs:139,164; crates/boyko_ecs/src/ecs/core/component/observers/trigger.rs:170,176; crates/boyko_physics/src/soft/component.rs:71,73,75,83,85,87,89,91,93,95,98,105,107,109,111,115,118,327,328,329,330,331,374,499,500,501,511,512,513,514,516,517,524,526,540,541
- **Evidence:**
  - D:/claude/BoykoEngine/docs/physics/ADVANCED-PHYSICS-DESIGN-SPACE.md:807 "component-owned sub-columns, as a resource-owned ROW BANK with per-body"
  - D:/claude/BoykoEngine/docs/physics/ADVANCED-PHYSICS-DESIGN-SPACE.md:833 "**The free-list of ranges is itself durable data and gets a home**"
  - D:/claude/BoykoEngine/docs/physics/ADVANCED-PHYSICS-DESIGN-SPACE.md:838 "**`SoftBody` becomes a `#[component(storage = "dense")]` handle**"
  - D:/wt/joltab/docs/DENSE-COMPONENTS-PLAN.md:56 "coloring DEPENDS on absolute body slot values"
  - crates/boyko_ecs/src/ecs/core/component/observers/mod.rs:155 "/// Stored as a field on `ArchetypeMaster`. Mutated only under `&mut self`"
  - crates/boyko_ecs/src/ecs/core/component/observers/mod.rs:156 "/// (`add` / `remove`), read under `&self` (the fire loop, `has_observer`, and"
  - crates/boyko_physics/src/resources.rs:693 "CSR offsets."
  - crates/boyko_physics/src/resources.rs:694 "cell_start: ScratchColumn&lt;u32>,"
  - crates/boyko_physics/src/resources.rs:696 "cell_bodies: ScratchColumn&lt;u32>,"
  - D:/wt/reflect:docs/REFLECTION-PLAN-CORE.md:3800 "as *mut String, s.to_owned())`** on the original arena `*mut` provenance"
- **Group notes:**
  - (physics-scene-math) This is the inventory's RaggedColumn restated as an ECS capability over existing storage (ScratchColumn columns + dense handle + hooks), not a new column primitive beside ComponentPool.
  - (reflect-lane, note) C11's set_str performs one std-heap allocation per write into a `String` field. The plan's own census finds zero String fields in engine components (REFLECTION-PLAN-CORE.md:3806-3807). Decided: the Str accessor targets the engine's text carriers (inline bytes like UiName, or a KF-26 interned name id / KF-03 owned span), not std String. OVERTURNED only if a shipping component with a String field appears.
  - (reflect-lane, status) existing ledger features - the destination for PLANNED CORE rung C11 (Str), no row today

### KF-04 Durable resource column (lifetime + serialization)

- **Status:** active. **Physics needs it:** yes. **Kind:** capability.
- **Rev 2:** Rev-2 users: the physics `PairCache` (gap 2, design D5: persistent resource columns, double-buffered) and, for the lifetime half only, the ui-lane font glyph, cmap and kern CSR columns (never cleared, never serialized); the font and sheet records themselves became components on asset entities (W4). SoftBody no longer needs it: it went to K7.
- **Adds to the kernel:** Ratifies a Resource-owned `ComponentPool` column for DURABLE data: no per-step clear contract, optional change ticks, and participation in serialization (a SerPod blit plus entity remap, as dense stores already have). "A durable bank that serialization cannot see is a side store by another name."
- **Crates:** boyko_ecs, boyko_serialize, boyko_physics, boyko_scene
- **Plan:** ADVANCED-PHYSICS-DESIGN-SPACE KR-3 (= animation AK-2).
- **Merged from:** physics-scene-math `KF-durable-resource-column`; ui-lane `KF-04 Durable resource column (lifetime + serialization)`
- **Rows (10):** crates/boyko_physics/src/narrowphase/axis_cache.rs:128; crates/boyko_physics/src/solver/warm_start.rs:214; crates/boyko_scene/src/identity.rs:72,74,81,121; [ui-lane] crates/boyko_ui/src/text/font.rs:33,36,38,41
- **Evidence:**
  - D:/claude/BoykoEngine/docs/physics/ADVANCED-PHYSICS-DESIGN-SPACE.md:868 "KR-3"
  - boyko_ecs/src/ecs/core/component/scratch/scratch_column.rs:43 "pub struct ScratchColumn&lt;T: Copy> {"
  - boyko_serialize/src: 0 matches for `ScratchColumn` (rg, this session)
- **Group notes:**
  - (physics-scene-math) DeferredFree and the interner need only the durable-lifetime ratification, not serialization.
  - (ui-lane, note) New UI users of an existing ledger feature, not a new feature. The ledger's ui-input group routed the font tables through KF-01 alone; durability is the part KF-01's scratch contract (cleared per run) does not state.
  - (ui-lane, capability) Ratifies a Resource-owned kernel column for DURABLE data (no per-step clear contract). The ledger lists only physics users; the UI tables are users of the lifetime half only (fonts and sheets are registered by code at setup, so they need no serialization).

### KF-05 World scratch frames (re-entrant LIFO scratch for &mut paths)

- **Status:** decided. **Physics needs it:** no. **Kind:** capability.
- **Rev 2:** Backing decided: ScratchColumn (UL-D5); the design space's FrameArena is not built (the engine design deleted its EK13 for the same reason: mark/rewind is `ScratchBuildView::truncate` on columns). The reflect lane adds a planned user (EG6 add_default's cold fallback).
- **Adds to the kernel:** EcsMaster-owned typed ScratchColumns, handed out as re-entrant LIFO frames (mark on open, truncate on drop). The users are the kernel's `&mut` paths (structural migration, required-id expansion, clone/prefab worklists, trigger-broadcast DFS) and exclusive systems. It uses the same backing as a system's Local scratch.
- **Crates:** boyko_ecs; any crate with exclusive systems (boyko_ui, boyko_demo)
- **Plan:** The design space's FrameArena is the other admissible backing. ONE of the two serves, not both; rev 2 chose ScratchColumn.
- **Merged from:** ecs-storage `KF-world-scratch-frames`; reflect-lane `World scratch frames (re-entrant LIFO scratch for &mut paths)`
- **Rows (15):** crates/boyko_ecs/src/ecs/core/clone/deep.rs:88,94,137,246; crates/boyko_ecs/src/ecs/core/clone/map.rs:22; crates/boyko_ecs/src/ecs/core/clone/materialize.rs:861,900; crates/boyko_ecs/src/ecs/core/clone/prefab.rs:458,460,608,679; crates/boyko_ecs/src/ecs/core/component/enable/enable_store.rs:769; crates/boyko_ecs/src/ecs/core/ecs_master/observer_api.rs:636,637,681
- **Evidence:**
  - crates/boyko_ecs/src/ecs/core/component/scratch/scratch_column.rs:43 "pub struct ScratchColumn&lt;T: Copy> {"
  - crates/boyko_ecs/src/ecs/core/component/scratch/views.rs:130 "pub fn clear(&mut self) {"
  - crates/boyko_ecs/src/ecs/core/component/scratch/views.rs:141 "pub fn push(&mut self, value: T) -> u32"
  - D:/claude/BoykoEngine/docs/memory/ALLOCATOR-DESIGN-SPACE.md:112 "\| **Frame** \| `FrameArena` + `FrameVec<T>`, `FrameSlice<T>`"
  - D:/wt/reflect:docs/REFLECTION-PLAN-ECS.md:1665 "`MaybeUninit` scratch with its release size/align assert and `#[cold]` heap fallback; U4."
- **Group notes:**
  - (ecs-storage) The design space's FrameArena is the other admissible backing for system-scratch; ONE of the two should serve, not both plus a third (ScratchStack).
  - (reflect-lane, note) EG6 add_default builds T::default() in a scratch BEFORE the migration (unwind safety: a panicking Default must leave no half-committed row) and falls back to the std heap for a component larger than the stack scratch. Decided: the fallback is a KF-05 world scratch frame (aligned, address-stable, kernel-owned, on the &mut EcsMaster path), never the std heap. OVERTURNED if EG6's own measurement (max size/align over every registered ComponentLayout, REFLECTION-PLAN-ECS.md:1702-1703) fits the stack scratch - then the fallback is deleted outright.
  - (reflect-lane, status) existing ledger feature - NEEDED by a PLANNED lane rung (EG6), no row today

### KF-06 Byte column (bulk append + fmt::Write)

- **Status:** active. **Physics needs it:** no. **Kind:** API on existing storage.
- **Rev 2:** The reflect lane adds a planned user (BOUNDARY B1 `VecSink` over a ScratchColumn&lt;u8>).
- **Adds to the kernel:** Byte-record append on existing kernel columns: `reserve_uninit(n) -> *mut u8` plus `set_len` on `VmColumn<MaybeUninit<u8>>` (kernel users) and on an exported `ScratchColumn<u8>` build view (other crates). Also `core::fmt::Write` on `ScratchBuildView<u8>`, so every serializer and save path writes into a kernel column.
- **Crates:** boyko_ecs (provides); boyko_serialize, boyko_render, boyko_ui, boyko_input
- **Plan:** The same capability as ALLOCATOR-DESIGN-SPACE `ByteColumn` (quoted in evidence). The per-queue resident floor depends on the packing plan (rung 0).
- **Merged from:** ecs-services `KF-byte-column`; ui-input `KF6-fmt-write-on-byte-column`; ui-lane `KF-06 Byte column (bulk append + fmt::Write)`; reflect-lane `Byte column (bulk append + fmt::Write) / Serialize seam on kernel columns`
- **Rows (13):** crates/boyko_ecs/src/ecs/core/asset/server.rs:127; crates/boyko_ecs/src/ecs/core/commands/command_queue.rs:83,85; crates/boyko_ecs/src/ecs/core/events/erased_buffer.rs:109; crates/boyko_ecs/src/ecs/core/serialize/mod.rs:81; crates/boyko_input/src/persist/keyname.rs:215,249; crates/boyko_input/src/persist/writer.rs:30,57,58; [ui-lane] crates/boyko_ui/src/reload/reconcile.rs:665; [ui-lane] crates/boyko_ui/src/text/serialize.rs:32
- **Evidence:**
  - D:/claude/BoykoEngine/docs/memory/ALLOCATOR-DESIGN-SPACE.md:142 "pub type ByteColumn = VmColumn&lt;MaybeUninit&lt;u8>>;   // + spare_ptr(additional) -> NonNull&lt;u8>, unsafe set_len(n)"
  - crates/boyko_ecs/src/ecs/core/component/scratch/scratch_column.rs:307 "pub(crate) fn set_len(&mut self, new_len: usize) {"
  - crates/boyko_ecs/src/ecs/core/component/scratch/scratch_column.rs:313 "pub(crate) fn grow_to(&mut self, rows: usize) -> bool {"
  - crates/boyko_ecs/src/ecs/memory/vm_column.rs:122 "/// Creates a LAZY column reserving room for `reserve_elems` elements"
  - boyko_ecs/src/ecs/core/component/scratch/views.rs:164 "pub fn extend_from_slice(&mut self, values: &[T])"
  - D:/wt/ui:crates/boyko_ecs/src/ecs/core/component/scratch/views.rs:93 "pub fn extend_from_slice(&mut self, values: &[T])"
  - D:/wt/reflect:docs/REFLECTION-PLAN-BOUNDARY.md:278 "One concrete pair ships: `VecSink` / `SliceSource` over a caller-provided `&mut Vec<u8>` /"
- **Group notes:**
  - (ecs-services) Same capability as the design's ByteColumn alias. Per-queue resident floor is a rung-0 (packing plan) dependency.
  - (ui-input) A trait impl on an existing primitive, not a new primitive (replaces ByteSink, R4).
  - (ui-lane, note) A trait impl on an existing primitive, not a new primitive (replaces ByteSink, R4).
  - (ui-lane, capability) core::fmt::Write implemented on ScratchBuildView&lt;'_, u8> (UTF-8 kept by construction since only &str is written), so every serializer / save path writes into a kernel column.
  - (reflect-lane, note) Decided: VecSink becomes a sink over a caller-owned ScratchColumn&lt;u8> (KF-06), the same answer the ledger gives boyko_serialize's SaveCursor (KF-08). Same contiguous bytes, so no performance cost to overturn.
  - (reflect-lane, status) existing ledger features - NEEDED by PLANNED BOUNDARY rung B1, no row today

### KF-07 Erased record column (heterogeneous drop-aware records)

- **Status:** active. **Physics needs it:** no. **Kind:** capability (the one surviving facility).
- **Adds to the kernel:** An address-stable, drop-aware record column for heterogeneous `'static` values: a Layout-aligned slot plus a `&'static` vtable, addressed by a thin handle, growing in place. It generalises CommandQueue's packed erased-closure records, so commands and kernel-owned trait objects share one record format. It was kept only after every ECS form was refuted for heterogeneous `dyn System` objects (MAX_COMPONENTS = 512 &lt; MAX_SYSTEMS_PER_SCHEDULE = 1024, and a dense column holds one type).
- **Crates:** boyko_ecs
- **Plan:** None. ALLOCATOR-RESEARCH records that the pre-X.J Layout-keyed arena was deleted for lack of clients; these rows plus CommandQueue are its clients.
- **Merged from:** ecs-schedule `KF-erased-record-column`
- **Rows (6):** crates/boyko_ecs/src/ecs/core/schedule/schedule.rs:227; crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs:183,837; crates/boyko_ecs/src/ecs/core/schedule/system_box.rs:83,129; crates/boyko_ecs/src/ecs/core/schedule/system_config.rs:189
- **Evidence:**
  - crates/boyko_ecs/src/ecs/core/commands/command_queue.rs:83 "pub(crate) bytes: Vec&lt;MaybeUninit&lt;u8>>,"
  - D:/claude/BoykoEngine/docs/memory/ALLOCATOR-RESEARCH.md:699 "a `Layout`-keyed variable-size arena"
  - crates/boyko_ecs/src/ecs/core/component/component_registry/mod.rs:63 "pub const MAX_COMPONENTS: usize = 512;"
  - crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs:71 "pub const MAX_SYSTEMS_PER_SCHEDULE: usize = 1024;"
- **Group notes:**
  - (ecs-schedule) Kept after refuting every ECS form for heterogeneous system objects (see primitives_refuted: DynArena). The allocator research records that the pre-X.J Layout-keyed arena was deleted as client-less; these rows plus CommandQueue are its clients. Other groups' `Box<dyn>` sites could use it too; that is not counted here.

### KF-08 Serialize seam on kernel columns

- **Status:** active. **Physics needs it:** no. **Kind:** type change.
- **Rev 2:** The reflect lane adds a planned user (BOUNDARY B1, same answer as SaveCursor).
- **Adds to the kernel:** (a) `SaveCursor` appends into a `ScratchColumn<u8>` build view instead of `&'a mut Vec<u8>`. (b) `LoadColumn` carries `(offset, len)` into the file bytes and derives Copy, so load instructions become rows a ScratchColumn can hold.
- **Crates:** boyko_ecs (core/serialize), boyko_serialize
- **Plan:** Conflicts with SERIALIZATION-PLAN (`SaveCursor` over a preallocated `Vec<u8>`).
- **Merged from:** codec-tools `KF-serialize-seam-on-kernel-columns`; reflect-lane `Byte column (bulk append + fmt::Write) / Serialize seam on kernel columns`
- **Rows (9):** crates/boyko_ecs/src/ecs/core/serialize/mod.rs:81; crates/boyko_serialize/src/load.rs:542,577,579,581; crates/boyko_serialize/src/save.rs:74,162,207,715
- **Evidence:**
  - boyko_ecs/src/ecs/core/serialize/mod.rs:91 "pub fn new(out: &'a mut Vec&lt;u8>) -> Self {"
  - boyko_ecs/src/ecs/core/serialize/mod.rs:81 "out: &'a mut Vec&lt;u8>,"
  - boyko_ecs/src/ecs/core/serialize/load_writer.rs:119 "pub enum LoadColumn&lt;'a> {"
  - boyko_ecs/src/ecs/core/component/scratch/scratch_column.rs:43 "pub struct ScratchColumn&lt;T: Copy> {"
  - boyko_ecs/src/ecs/core/asset/backing.rs:115 "pub fn register_asset_layout&lt;T: 'static>(drop_fn: Option&lt;DropFn>) -> ComponentId {"
  - D:/wt/reflect:docs/REFLECTION-PLAN-BOUNDARY.md:278 "One concrete pair ships: `VecSink` / `SliceSource` over a caller-provided `&mut Vec<u8>` /"
- **Group notes:**
  - (reflect-lane, note) Decided: VecSink becomes a sink over a caller-owned ScratchColumn&lt;u8> (KF-06), the same answer the ledger gives boyko_serialize's SaveCursor (KF-08). Same contiguous bytes, so no performance cost to overturn.
  - (reflect-lane, status) existing ledger features - NEEDED by PLANNED BOUNDARY rung B1, no row today

### KF-09 Loader decode context

- **Status:** active. **Physics needs it:** no. **Kind:** capability.
- **Adds to the kernel:** `AssetLoader::decode` receives an ECS-owned decode context: reusable ScratchColumn lanes for per-decode transients, plus per-type staging payload lanes that the decoded bytes are written into. `Asset::Cpu` becomes a Copy `{start, len}` range record. The codec half: `decode_png` splits into a header probe and a decode-into-slice call, and `boyko_image` keeps its no-workspace-crate edge.
- **Crates:** boyko_ecs, boyko_render, boyko_image, boyko_ui, boyko_fontbake
- **Plan:** None.
- **Merged from:** render `KF-loader-decode-context`; codec-tools `KF-loader-decode-context`; ecs-services `KF-loader-decode-context`
- **Rows (65):** crates/boyko_ecs/src/ecs/core/asset/server.rs:127; crates/boyko_ecs/src/ecs/core/asset/staging.rs:58; crates/boyko_render/src/loaders/glb.rs:94,95,96,211,213,264,286,561,562,693,695,699,704,714,718,720,726,727,786,831,857,915,916; crates/boyko_render/src/loaders/obj.rs:66,67,68,69,78,129,138,183,184,187,188; crates/boyko_render/src/loaders/png_texture.rs:42,50; crates/boyko_render/src/mesh_data.rs:28,30; crates/boyko_render/src/tangent.rs:62,63; crates/boyko_render/src/texture.rs:806,808,828; crates/boyko_render/src/texture_data.rs:28; crates/boyko_fontbake/src/atlas.rs:116,629; crates/boyko_image/src/inflate.rs:213,466,488,517,557,650,674; crates/boyko_image/src/png.rs:60,81,144,378,381,421,430,441,456
- **Evidence:**
  - boyko_ecs/src/ecs/core/asset/loader.rs:27 "fn decode(bytes: &[u8]) -> Result&lt;&lt;Self::Out as Asset>::Cpu, AssetError>;"
  - boyko_ecs/src/ecs/core/asset/asset.rs:34 "type Cpu: Send + 'static;"
  - boyko_ecs/src/ecs/core/asset/staging.rs:58 "queue: Vec&lt;Staged&lt;A>>,"
  - boyko_ecs/src/ecs/core/asset/server.rs:133 "staging.push(Staged { handle, cpu });"
  - boyko_image/src/png.rs:131 "let final_size = (info.width as usize)"
  - boyko_image/src/png.rs:127 "let expected_len = scanline_stride"
  - boyko_image/Cargo.toml:5 "it takes no other workspace crate."
  - boyko_render/src/gpu_upload.rs:114 "for staged in staging.drain() {"
  - crates/boyko_ecs/src/ecs/core/asset/asset.rs:34 "type Cpu: Send + 'static;"
- **Group notes:**
  - (render) Threadpool decode (rung A5) would add per-worker reserved ranges; that is not needed while decode stays synchronous (server.rs:127-133).
  - (ecs-services) Deduplicate with render.ecsform.json.

### KF-10 Structured asset error

- **Status:** active. **Physics needs it:** no. **Kind:** type change.
- **Adds to the kernel:** `AssetError::Decode` / `::Io` carry a `boyko_log` code, a `&'static str` and POD args instead of a `String`. Formatting happens at emission. The home of the datum is the kernel diagnostics substrate.
- **Crates:** boyko_ecs, boyko_log, boyko_render, boyko_image
- **Plan:** Agrees with LOGGING-SYSTEM-PLAN Decision 1 (deferred formatting) and with the error-codes line of ALLOCATOR-DESIGN-SPACE.
- **Merged from:** render `KF-structured-asset-error`; ecs-services `KF-structured-asset-error`
- **Rows (77):** crates/boyko_ecs/src/ecs/core/asset/error.rs:18,23; crates/boyko_ecs/src/ecs/core/asset/server.rs:139; crates/boyko_render/src/loaders/glb.rs:78,163,168,172,174,185,190,194,202,208,215,220,235,237,240,246,254,255,262,279,284,306,321,326,390,392,399,401,404,406,409,414,418,420,429,437,439,494,496,516,520,735,746,789,792,796,798,804,811,815,852,855,861,886,891,903,921,923; crates/boyko_render/src/loaders/obj.rs:64,91,132,270,278,285,292,299,305; crates/boyko_render/src/loaders/png_texture.rs:25,71; crates/boyko_render/src/loaders/ron_material.rs:38,91,97,101,110
- **Evidence:**
  - boyko_ecs/src/ecs/core/asset/error.rs:23 "Decode(String),"
  - boyko_ecs/src/ecs/core/asset/server.rs:171 "boyko_log::codes::E0801,"
  - D:/wt/joltab/docs/LOGGING-SYSTEM-PLAN.md:150 "### Decision 1: Deferred formatting"
  - crates/boyko_ecs/src/ecs/core/asset/server.rs:171 "boyko_log::codes::E0801,"
  - crates/boyko_ecs/src/ecs/core/asset/server.rs:172 ""asset load failed for '{}': {}","
- **Group notes:**
  - (render) Agrees with D:/claude/BoykoEngine/docs/memory/ALLOCATOR-DESIGN-SPACE.md:361 "error types carry codes (`boyko_log/src/codes.rs` already exists) + `&'static str`". It differs from that doc only in not needing HeapString for these sites.
  - (ecs-services) error.rs:30 / server.rs:184 go further: the extension field is dropped because the log line already prints the path.

### KF-11 Relation reverse index on kernel storage (K7 spans)

- **Status:** decided. **Physics needs it:** yes. **Kind:** capability. **Physics design:** K7. **Engine design:** EK15c.
- **Rev 2:** Physics: yes (gap 9: "the joints of this body" is a one-to-many reverse index). Backing decided (section Decisions): per-target spans of a K7 segmented column, the engine design's EK15c, NOT the intrusive links rev 1 proposed. Traversal reads one contiguous span; links cost one random access per child. main:docs/unification/ENGINE-RUNTIME-ECS-DESIGN.md:2547 "> \| EK15c **(rev 3)** \| Relation storage on K7 \|". Overturned by: a reparent-churn bench where links beat spans beyond band while the propagation and layout traversal benches stay within band.
- **Adds to the kernel:** A `RelationshipSourceCollection` backing that stores the one-to-many reverse index as fixed-size links in component columns: `{first, last, len}` in the target, and `{prev, next}` in a kernel-owned dense link component on each source. add and remove are O(1) and take the world. Every relation, `Children` included, gets it through the derive's Collection type. Physics joint and island edges would use the same feature.
- **Crates:** boyko_ecs, boyko_macros (relationship derive), boyko_scene, boyko_ui
- **Plan:** RELATIONS-API-PLAN leaves the backing swappable. MEMORY-SYSTEM-AUDIT asks for a traversal measurement first (not measured).
- **Merged from:** ecs-services `KF-relation-linked-reverse-index`; ecs-storage `KF-relation-collection-on-kernel-storage`
- **Rows (4):** crates/boyko_ecs/src/ecs/core/component/observers/entity_store.rs:102; crates/boyko_ecs/src/ecs/core/hierarchy/mod.rs:120,159; crates/boyko_ecs/src/ecs/core/relationship/collection.rs:85
- **Evidence:**
  - crates/boyko_ecs/src/ecs/core/hierarchy/mod.rs:120 "pub struct Children(Vec&lt;Entity>);"
  - crates/boyko_ecs/src/ecs/core/hierarchy/mod.rs:87 "pub struct ChildOf(pub Entity);"
  - crates/boyko_macros/src/relationship.rs:640 "type Collection = #field_ty;"
  - crates/boyko_ecs/src/ecs/core/relationship/collection.rs:4 "//! The cardinality (one-to-many vs 1:1) and the backing store (`Vec` today,"
  - crates/boyko_ecs/src/ecs/core/relationship/mod.rs:528 "fn apply(self, world: &mut EcsMaster) {"
  - crates/boyko_ecs/src/ecs/core/relationship/mod.rs:689 "fn apply(self, world: &mut EcsMaster) {"
- **Group notes:**
  - (ecs-services) Traversal cost against the contiguous Vec is NOT measured. MEMORY-SYSTEM-AUDIT.md:87 asks for exactly that measurement first. Physics joint / island membership edges (other groups) would ride the same feature.
  - (ecs-storage) Its own rows belong to the relationship/hierarchy group, not to ecs-storage; listed here as a dependency only.

### KF-12 Multi-target relation (OPTIONAL)

- **Status:** active. **Physics needs it:** no. **Kind:** capability.
- **Adds to the kernel:** One source holds several edges of one relation kind, so InSet, Before/After and RunIf need no edge entities. OPTIONAL: Relations v1 can already express all of these with edge entities, at one entity per edge.
- **Crates:** boyko_ecs
- **Plan:** Relations v1 Decision 1 (single target).
- **Merged from:** ecs-schedule `KF-multi-target-relation`
- **Rows (18):** crates/boyko_ecs/src/ecs/core/schedule/schedule.rs:161,172; crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs:116,122,127,139,155,156,157,159,683,693; crates/boyko_ecs/src/ecs/core/schedule/system_descriptor.rs:50,55,62,80,81,82
- **Evidence:**
  - crates/boyko_ecs/src/ecs/core/relationship/mod.rs:200 "/// pointing at one target (Relations v1, Decision 1)."
  - crates/boyko_ecs/src/ecs/core/relationship/collection.rs:80 "impl RelationshipSourceCollection for Vec&lt;Entity> {"
- **Group notes:**
  - (ecs-schedule) OPTIONAL: Relations v1 can express every one of these rows with edge entities carrying one single-target relation per endpoint. Recorded because the edge-entity form costs one entity per edge.

### KF-13 Default-excluded (hidden) entities; prefab templates as entities

- **Status:** decided. **Physics needs it:** no. **Kind:** capability. **Engine design:** EK18.
- **Rev 2:** Decided yes (section Decisions); the engine design's EK18 (main:docs/unification/ENGINE-RUNTIME-ECS-DESIGN.md:913 "\| EK18 \| Default-excluded prefab entities \| marker that matches no query unless named \| services KF7 \|").
- **Adds to the kernel:** A default-excluded marker: an entity that carries it matches no query unless the query names it (Bevy's disabled-by-default filter, flecs' Prefab). Prefab templates then live as entities and instantiate as `clone_subtree` plus marker removal. The SAME marker is the internal-entity guard that KF-14 and KF-15 need, hidden from user queries, serialization and despawn-all.
- **Crates:** boyko_ecs, boyko_scene (prefab gate tests)
- **Plan:** None. It conflicts with the Principle-0 exception that prefab.rs:28-29 claims for itself (quoted in evidence).
- **Merged from:** ecs-storage `KF-prefab-entities`
- **Rows (8):** crates/boyko_ecs/src/ecs/core/clone/prefab.rs:136,281,284,364,458,460,608,679
- **Evidence:**
  - crates/boyko_ecs/src/ecs/core/clone/prefab.rs:28 "This is the legitimate "transient/template" exception to"
  - crates/boyko_ecs/src/ecs/core/clone/prefab.rs:29 "Principle 0, documented as such."
  - crates/boyko_ecs/src/ecs/core/clone/prefab.rs:44 "//!   `clone_subtree`, which re-materializes dense): a prefab of a dense-physics-body"
  - crates/boyko_ecs/src/ecs/core/clone/prefab.rs:45 "//!   entity instantiates WITHOUT the dense membership. A capture-time"

### KF-14 System entities

- **Status:** decided. **Physics needs it:** no. **Kind:** entity-model change (decided in rev 2).
- **Rev 2:** Decided yes (section Decisions): systems are hidden entities; the compiled executor tables stay kernel-internal behind SystemIndex -> entity, so the per-frame dispatch path is unchanged.
- **Adds to the kernel:** Systems, condition systems and system sets become hidden entities of the world the Schedule is bound to. SystemBox becomes a dense component, GpuAccessIntent and set names become components, and InSet, Before/After and RunIf become relations. The compiled executor tables stay kernel-internal behind a SystemIndex->entity table.
- **Crates:** boyko_ecs, boyko_render (gpu intent), boyko_app; the API for other crates is unchanged
- **Plan:** None. Rev 1 called this the one owner decision the scheduler raises; rev 2 decides it. PHASE-15 research chose Bevy for ORDERING semantics only.
- **Merged from:** ecs-schedule `KF-system-entities`
- **Rows (26):** crates/boyko_ecs/src/ecs/core/schedule/schedule.rs:122,161,172; crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs:106,116,122,127,131,139,153,155,156,157,158,159,682,683,693; crates/boyko_ecs/src/ecs/core/schedule/system_descriptor.rs:50,55,62,80,81,82; crates/boyko_ecs/src/ecs/core/system/system_meta.rs:140,331
- **Evidence:**
  - crates/boyko_ecs/src/ecs/core/schedule/schedule.rs:216 "pub(crate) world_id: WorldId,"
  - crates/boyko_ecs/src/ecs/core/ecs_master/observer_api.rs:306 "pub fn observe&lt;E: Trigger>(&mut self, runner: TriggerFn) -> ObserverId {"
  - crates/boyko_ecs/src/ecs/core/app/app.rs:140 "schedule: Option&lt;Schedule>,"
  - D:/wt/joltab/docs/archive/PHASE-15-RESEARCH.md:29 "Because boyko is task-parallel, **Bevy is the correct reference model**, not flecs."
- **Group notes:**
  - (ecs-schedule) This is the ONE owner decision this group raises. Earlier research picked Bevy over flecs as the reference for ORDERING semantics (task- vs data-parallel), which does not decide where system data lives. Every row that needs this feature records its fallback: kernel-internal on a Schedule-owned ComponentPool/VmColumn, which still needs no new primitive.

### KF-15 Observer entities

- **Status:** decided. **Physics needs it:** no. **Kind:** entity-model change (decided in rev 2).
- **Rev 2:** Decided yes (section Decisions): observers are entities; the fire path reads a derived contiguous dispatch index, not one component per observer.
- **Adds to the kernel:** Observers become entities with an Observer component. Entity-targeted observers link to the observed entity through an `Observes` relation, whose despawn cascade replaces the generation recycle guard. The global dispatch tables become derived indexes.
- **Crates:** boyko_ecs (no other crate calls `observe_entity*`)
- **Plan:** None.
- **Merged from:** ecs-storage `KF-observer-entities`
- **Rows (5):** crates/boyko_ecs/src/ecs/core/component/observers/entity_store.rs:102,111,120,123,125
- **Evidence:**
  - crates/boyko_ecs/src/ecs/core/component/observers/mod.rs:59 "pub struct ObserverId(pub(crate) u64);"
  - crates/boyko_ecs/src/ecs/core/relationship/mod.rs:4 "ANY user struct can declare a one-to-many bidirectional"
  - crates/boyko_ecs/src/ecs/core/relationship/mod.rs:5 "relation maintained by the existing component-hook substrate"
  - crates/boyko_ecs/src/ecs/core/component/observers/entity_store.rs:96 "/// despawn+reuse bumps the live generation; a stale list whose generation no"
  - crates/boyko_ecs/src/ecs/core/component/observers/entity_store.rs:97 "/// longer matches is reclaimed and never fires (the recycle guard)."
- **Group notes:**
  - (ecs-storage) No crate outside boyko_ecs calls observe_entity* in this tree (rg this session).

### KF-16 Enable write inside iteration

- **Status:** withdrawn. **Physics needs it:** no. **Kind:** capability.
- **Rev 2:** WITHDRAWN in rev 2. Its only user was the physics Sleeping write-back, and physics Q3 / D4 put sleep in the `BodyGate` group column instead (writer change W3). main:docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md:504 "**Not needed:** an in-place `EnableMut` toggle. The dense `free: Vec<u32>`".
- **Adds to the kernel:** A query data term (`EnableMut<T>` / `SetEnabled<T>`) that flips the CURRENT row's EnableTag bit during iteration. The scheduler accounts it as a write to T's enable column; there is no Command and no entity lookup.
- **Crates:** boyko_ecs, boyko_physics (the Sleeping write-back in physics_apply)
- **Plan:** None.
- **Merged from:** physics-scene-math `KF-enable-write-in-iteration`
- **Rows that named it before rev 2 (2; none needs it now):** crates/boyko_physics/src/resources.rs:3057,3104
- **Evidence:**
  - boyko_ecs/src/ecs/core/ecs_master/enable_tag_api.rs:88 "pub fn enable&lt;T: Component>(&mut self, entity: Entity) {"
  - boyko_ecs/src/ecs/core/system/params/entity_commands.rs:220 "pub fn enable&lt;T: Component>(&mut self) -> &mut Self {"
  - boyko_physics/src/systems.rs:207 "IsEnabled&lt;Simulated>,"
- **Group notes:**
  - (physics-scene-math) Without it the latch write-back needs KR-1's row->entity map plus one command per flip; flips are rare, so this is a paradigm feature (one uniform mutation path for per-row bits), not a speed lever.

### KF-17 Enable initial polarity

- **Status:** active. **Physics needs it:** no. **Kind:** capability. **Engine design:** EK4.
- **Rev 2:** The engine design's EK4 (main:docs/unification/ENGINE-RUNTIME-ECS-DESIGN.md:896 "\| EK4 \| Enable initial polarity \| `default_enabled` \| KF-enable-initial-polarity \| `LightEnabled` \| here \|").
- **Adds to the kernel:** An EnableTag declares its default state. The store keeps the complement bit, so a never-toggled row reads ENABLED at zero cost with no page allocation. It uses the inverted match path the store already has.
- **Crates:** boyko_ecs, boyko_macros, boyko_render; other bitset tags (physics Simulated, UI) not checked
- **Plan:** None.
- **Merged from:** render `KF-enable-initial-polarity`
- **Rows (8):** crates/boyko_render/src/light_system.rs:729,733,736,739,742,745,748,751
- **Evidence:**
  - boyko_ecs/src/ecs/core/component/enable/enable_store.rs:204 "4096-row range) reads as `false` (all-disabled). Hot read path."
  - boyko_ecs/src/ecs/core/component/enable/enable_store.rs:376 "SET. `invert == true` (Disabled / `without_enabled`): a row matches iff its"
  - boyko_render/src/light.rs:244 "/// A never-toggled row reads DISABLED (the bitset default). To keep pre-existing /"
  - boyko_macros/src/component.rs:326 "a bitset tag has no `ComponentPool` and must not be"
- **Group notes:**
  - (render) Other bitset tags (physics Simulated, boyko_ui tags) might want the same declaration; not checked.

### KF-18 Dense x enable iteration (existing plan)

- **Status:** withdrawn. **Physics needs it:** no. **Kind:** existing plan.
- **Rev 2:** WITHDRAWN in rev 2: it was needed only if bodies went dense while Sleeping stayed an EnableTag, and physics Q3 / D4 removed the EnableTag. `Simulated` / `Kinematic` stay bitset tags read once per row at S1 in table order, not on the dense stride.
- **Adds to the kernel:** Queries that combine a dense include with an enable term, on the pure-dense stride. Needed ONLY if bodies become dense (Stage P) while Sleeping stays an enable tag. Today that combination is a compile-time reject.
- **Crates:** boyko_ecs, boyko_physics
- **Plan:** docs/DENSE-ENABLE-QUERY-PLAN.md.
- **Merged from:** physics-scene-math `KF-dense-enable-iteration (existing plan)`
- **Rows that named it before rev 2 (2; none needs it now):** crates/boyko_physics/src/resources.rs:3057,3104
- **Evidence:**
  - boyko_ecs/tests/enable_filter_compile_fail/query_dense_iter_mut_enable_rejected.rs:27 "const _: () = assert_dense_iter_no_enable::&lt;&mut Dense, Disabled&lt;Tag>>();"
  - D:/wt/joltab/docs/DENSE-COMPONENTS-PLAN.md:26 "**Physics (Stage P)**: RigidBody*/velocity→dense; contacts→dense slots"
- **Group notes:**
  - (physics-scene-math) Conditional. The alternative after Stage P is the latch as a bit inside a dense SleepState keyed by the body slot.

### KF-19 Dense slot access (typed, scheduler-visible)

- **Status:** active. **Physics needs it:** yes. **Kind:** capability. **Physics design:** K3.
- **Rev 2:** Realised by the physics design's K3 dense groups: `DenseColumn<T>` / `DenseColumnMut<T>` params and typed views with disjoint `range_mut` (design section 8).
- **Rev 3:** Rev 3 (item 2): the asset K3 groups are users. The boot backfill and the two teardown handle lists walk the group column in place (`gpu_upload.rs:216`, `mesh_assets.rs:582`, `texture.rs:709`).
- **Adds to the kernel:** Read side: `Query<&T>::dense_slots() -> DenseSlots<'_, T>` (Copy + Send + Sync, `get(slot)`), which can be captured into a par_iter body to read OTHER entities' rows by slot. Write side: sequential `dense_slot_mut(slot) -> Mut<T>`, which stamps the per-slot tick. This is the typed face of `DenseSolveView::row_ptr`, which physics Stage P needs in place of raw pointers.
- **Crates:** boyko_ecs, boyko_demo, boyko_physics (Stage P)
- **Plan:** DENSE-COMPONENTS-PLAN Stage P (partially).
- **Merged from:** app-demo `KF-dense-slot-access`
- **Rows (14):** crates/boyko_render/src/gpu_upload.rs:216; crates/boyko_render/src/mesh_assets.rs:582; crates/boyko_render/src/texture.rs:709; crates/boyko_demo/src/sim/grid.rs:41; crates/boyko_demo/src/sim/resources.rs:134,142,191,193,195,199,211,212,213,214
- **Evidence:**
  - crates/boyko_ecs/src/ecs/core/component/dense/views.rs:239 "pub unsafe fn row_ptr(&self, slot: usize) -> *mut u8 {"
  - crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs:590 "pub fn dense_registry(&self) -> &crate::ecs::core::component::dense::DenseRegistry {"
  - crates/boyko_ecs/src/ecs/core/iters/query/query.rs:460 "pub fn dense_iter(&self) -> DenseQueryIter&lt;'_, D>"
  - crates/boyko_ecs/src/ecs/core/iters/query/query.rs:896 "pub fn get(&self, entity: Entity) -> Option&lt;D::Item&lt;'_>>"
  - docs/DENSE-COMPONENTS-PLAN.md:26 "10. **Physics (Stage P)**: RigidBody*/velocity→dense; contacts→dense slots (resolved at contact-build); DELETE the Vec mirrors; solver inner loop via `DenseSolveView::row_ptr`; 31-lane `ContactColumns` UNTOUCHED. AVX re-gather into a contiguous `ScratchColumn` is the DEFAULT (W4)."

### KF-20 Dense par_iter / par_for_each_chunk

- **Status:** active. **Physics needs it:** yes. **Kind:** capability. **Physics design:** K4.
- **Rev 2:** The physics design's K4 (KE15): `par_iter` / `par_for_each_chunk` over mixed table + dense terms. main:docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md:469 "\| K4 \| KE15: `par_iter` / `par_for_each_chunk` over mixed table + dense / `GroupSlot` terms \|".
- **Adds to the kernel:** par_iter / par_iter_mut / par_for_each_chunk over queries with dense terms. Today these are compile-rejected. This is intra-system parallelism over the dense kernel column, which is the owner's physics point.
- **Crates:** boyko_ecs, boyko_demo, boyko_physics
- **Plan:** None.
- **Merged from:** app-demo `KF-dense-par-iter`
- **Rows (6):** crates/boyko_demo/src/sim/resources.rs:191,193,195,211,212,213
- **Evidence:**
  - crates/boyko_ecs/src/ecs/core/iters/query/par_iter.rs:302 "// dense `D`/`F` is compile-rejected here — use the sequential `Query::iter`" / :307 "!D::HAS_DENSE && !F::HAS_DENSE,"
  - crates/boyko_ecs/src/ecs/core/iters/query/chunk_iter.rs:116 "!D::HAS_DENSE && !F::HAS_DENSE,"

### KF-21 In-scope barrier + ordered parallel emit

- **Status:** active. **Physics needs it:** yes. **Kind:** capability (no ledger row). **Physics design:** K5a, K5b.
- **Rev 2:** The physics design's K5a `par_range` and K5b gang `par_phases`, with the enforced nesting rule. main:docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md:471 "\| K5b \| Gang (`par_phases`) + task run context \| §10.1, including the nesting rule \| S2 grid, S3, S5, soft solve \|".
- **Adds to the kernel:** A barrier inside one pool scope (one scope per step instead of one per colour wave), plus a deterministic count/prefix/emit into a ScratchColumn through its solve view. Any stage can use it. It turns the serial physics stages into parallel systems over kernel columns.
- **Crates:** boyko_threadpool / boyko_ecs scheduler, boyko_physics
- **Plan:** The KE16 lane (OPEN-QUESTIONS, quoted in evidence).
- **Merged from:** physics-scene-math `KF-in-scope-barrier + ordered-parallel-emit (no ledger row)`
- **Rows (0):** none: a capability with no active ledger row
- **Evidence:**
  - boyko_physics/src/resources.rs:650 "const MIN_PARALLEL_BODIES: usize = 4096;"
  - boyko_physics/src/solver/colored.rs:227 "const MIN_PARALLEL_SLOTS_PER_COLOR: u32 = 256;"
  - D:/wt/joltab/docs/OPEN-QUESTIONS.md:5145 "The fix is one `pool.scope` per STEP instead of 72, which needs an in-scope BARRIER on"
- **Group notes:**
  - (physics-scene-math) No ledger row needs it; recorded because the brief ties the ECS-paradigm answer to the Jolt residual. physics_narrowphase is a single serial loop over pairs, the broadphase runs one chunk below 4096 bodies, and colours under 256 slots are solved inline.

### KF-22 Entity datum in Query (KR-1)

- **Status:** subsumed. **Physics needs it:** yes. **Kind:** capability (no ledger row). **Physics design:** K3.
- **Rev 2:** SUBSUMED in rev 2 by K3's `GroupSlot<G>` query datum and the group's `s2e` (physics S7 reads `DenseSlots<PhysicsBody>` to project slots to entities).
- **Adds to the kernel:** A query term that yields the row's Entity, so cross-frame per-body data that is not a component can be keyed by identity rather than by gather row, and solver rows can be projected to entities for contact events. Named by the physics entity model: "**Kernel request KR-1**: an entity datum in `Query`" (ADVANCED-PHYSICS-DESIGN-SPACE.md:183).
- **Crates:** boyko_ecs, boyko_physics
- **Plan:** ADVANCED-PHYSICS-DESIGN-SPACE KR-1.
- **Merged from:** the physics entity model (no group feature)
- **Rows that named it before rev 2 (0; none needs it now):** none

### KF-23 Event lane policies (lossless / drop-oldest / coalesce / non-system producers)

- **Status:** active. **Physics needs it:** yes. **Kind:** capability. **Engine design:** EK7 (in part).
- **Rev 2:** The physics contact events are ordinary kernel events written by S7 (D6). The OS-producer half is the engine design's EK7 `OsEventSink<E>` (main:docs/unification/ENGINE-RUNTIME-ECS-DESIGN.md:899 "\| EK7 \| Per-type event swap + OS sink \| `#[event(swap = …)]`; `OsEventSink<E>` \| host K1+K2 \|").
- **Adds to the kernel:** An overflow policy per event type, chosen at preregistration: reject-newest (today's EventBufferFull), drop-oldest ring, or grow-in-place (lossless). Also coalesce-in-place, and sends from producers that are not systems: hooks and observers (their DeferredEcsMaster handle offers only resource_mut and commands today), and an OS window procedure on the pump thread. Physics contact begin/end events need the lossless lane.
- **Crates:** boyko_ecs, boyko_scene, boyko_render, boyko_input, boyko_rhi_vulkan, boyko_physics
- **Plan:** None. The memory index names an "Event-lane hazard" campaign; this ledger did not read its interaction with this feature.
- **Merged from:** physics-scene-math `KF-lossless-hook-event`; ui-input `KF3-event-overflow-policy`; rhi `KF-drop-oldest-coalescing-event-lane`
- **Rows (5):** crates/boyko_scene/src/asset_refs.rs:99; crates/boyko_scene/src/propagation.rs:133; crates/boyko_rhi_vulkan/src/window.rs:122; crates/boyko_input/src/raw/queue.rs:35,59
- **Evidence:**
  - boyko_ecs/src/ecs/core/component/hooks/deferred_master.rs:105 "pub fn resource_mut&lt;R: Resource>(&mut self) -> Option&lt;&mut R> {"
  - boyko_ecs/src/ecs/core/events/event_config.rs:47 "if capacity_per_lane == 0 \|\| capacity_per_lane > MAX_EVENT_CAPACITY {"
  - boyko_ecs/src/ecs/core/events/event_buffer.rs:358 "return Err(EcsError::EventBufferFull {"
  - boyko_ecs/src/ecs/core/events/event_dispatcher.rs:124 "Events sent during frame N become readable via [`events`] only after the"
  - boyko_input/src/raw/queue.rs:18 "/// # Overflow policy: drop-oldest"
  - The kernel lane refuses the newest send when full (boyko_ecs/src/ecs/core/events/event_buffer.rs:62 "/// `boyko-W0701` — a write lane was full, so the send was refused."); the RHI ring (boyko_rhi_vulkan/src/window.rs:112 "/// Drop-oldest (mirroring `boyko_input::RawInputQueue`'s policy): on a slow") and boyko_input's RawInputQueue (boyko_input/src/raw/queue.rs:33 "pub struct RawInputQueue {") both hand-roll drop-oldest rings outside the kernel event system.
- **Group notes:**
  - (physics-scene-math) Also the natural producer seam for physics contact begin/end events (D:/claude/BoykoEngine/docs/physics/ADVANCED-PHYSICS-DESIGN-SPACE.md:1282 "**Contact begin/end and sensor EVENTS, plus the `Contact` producer**"), which tolerate next-frame visibility but not silent loss.
  - (ui-input) The non-worker sender lane the runner needs already exists (boyko_ecs/src/ecs/core/events/event_config.rs:39 "pool worker plus one for a non-worker sender; KE8).").

### KF-24 Same-frame event delivery

- **Status:** rejected. **Physics needs it:** no. **Kind:** capability.
- **Rev 2:** REJECTED in rev 2 (writer change W5): one-frame UI messages are triggers raised through `Commands::trigger` and applied in the producer's apply window, which the successor waits for; no same-frame event mode is built. main:docs/unification/ENGINE-RUNTIME-ECS-DESIGN.md:330 "- Click, submit and hover-enter/leave become `Trigger`s with `Up` propagation, raised through `Commands::trigger` (EK8)." ; main:docs/unification/ENGINE-RUNTIME-ECS-DESIGN.md:345 "- A same-frame event mode: rejected (rev 1 reasons stand).". UL-D4's "KF-24 is built" half is superseded; its row stays an event.
- **Rev 3:** Re-decided in rev 3 (item 3) against the one row that still asked for it, joltab `crates/boyko_scene/src/propagation.rs:133`, and REJECTED again with evidence. The detach observer runs in the producer's apply window, and the executor applies a completed system's commands and drains its deferred hooks before it decrements that system's ordered successors: joltab:crates/boyko_ecs/src/ecs/core/schedule/schedule.rs:810 "self.systems[i].system.apply(world);" ; joltab:crates/boyko_ecs/src/ecs/core/schedule/schedule.rs:817 "world.drain_deferred_hook_queue();" ; joltab:crates/boyko_ecs/src/ecs/core/schedule/schedule.rs:831 "self.executor_scratch.pred_remaining[s] -= 1;". So the observer-to-system queue is delivered at the ordering edge, the same frame, for any system ordered before propagation. A same-frame event mode would add a publish step at every edge for every event type; this edge costs one push per detach.
- **Adds to the kernel:** An event type's write lanes are published at a schedule ordering edge (after the producer, before its consumer), so a pair ordered within one frame sees the events that frame. Today they are visible only after the next update_events swap.
- **Crates:** boyko_ecs, boyko_ui, boyko_input, boyko_scene
- **Plan:** None.
- **Merged from:** ui-input `KF4-same-frame-event-delivery`; physics-scene-math `KF-lossless-hook-event`; ui-lane `KF-24 Same-frame event delivery`
- **Rows that named it before rev 2 (4; none needs it now):** crates/boyko_scene/src/asset_refs.rs:99; crates/boyko_scene/src/propagation.rs:133; crates/boyko_input/src/raw/queue.rs:35; [ui-lane] crates/boyko_ui/src/interaction/focus.rs:128
- **Evidence:**
  - boyko_ecs/src/ecs/core/events/event_dispatcher.rs:43 "/// Called by `update_events`. Reads write lanes, flattens to `reader_buf`,"
  - boyko_ecs/src/ecs/core/events/event_dispatcher.rs:931 "// Frame 1: send; not yet visible."
  - boyko_ecs/src/ecs/core/component/hooks/deferred_master.rs:105 "pub fn resource_mut&lt;R: Resource>(&mut self) -> Option&lt;&mut R> {"
  - boyko_ecs/src/ecs/core/events/event_config.rs:47 "if capacity_per_lane == 0 \|\| capacity_per_lane > MAX_EVENT_CAPACITY {"
  - boyko_ecs/src/ecs/core/events/event_buffer.rs:358 "return Err(EcsError::EventBufferFull {"
  - boyko_ecs/src/ecs/core/events/event_dispatcher.rs:124 "Events sent during frame N become readable via [`events`] only after the"
  - D:/wt/ui:crates/boyko_ecs/src/ecs/core/events/event_dispatcher.rs:43 "/// Called by `update_events`. Reads write lanes, flattens to `reader_buf`,"
  - D:/wt/ui:crates/boyko_ecs/src/ecs/core/events/event_dispatcher.rs:912 "// Frame 1: send; not yet visible."
- **Group notes:**
  - (ui-input) Without it hover_entered stays system-scratch (an event would add one frame of OnHover latency). The owner's memory index lists an "Event-lane hazard" campaign; its interaction with this feature was not read here.
  - (physics-scene-math) Also the natural producer seam for physics contact begin/end events (D:/claude/BoykoEngine/docs/physics/ADVANCED-PHYSICS-DESIGN-SPACE.md:1282 "**Contact begin/end and sensor EVENTS, plus the `Contact` producer**"), which tolerate next-frame visibility but not silent loss.
  - (ui-lane, note) Without it hover_entered stays system-scratch (an event would add one frame of OnHover latency). The owner's memory index lists an "Event-lane hazard" campaign; its interaction with this feature was not read here. \| ui-lane: DECIDED 2026-09-11 (section (i) 'hover_entered'): event, and KF-24 is built. Performance: an event is one lane append per hover transition and a same-frame read; the command channel cannot carry it without inserting and removing a marker component (two structural migrations per hover transition), and an enable-state bit needs a clear pass over last frame's bits. Until KF-24 exists the row stays system-scratch (one frame of OnHover latency is a behaviour regression, not a cost). NOT needed by the lane's UiTweenScratch::done: the command channel already delivers at the ordering edge (see that row).
  - (ui-lane, capability) Publishing an event type's write lanes at a schedule ordering edge (after the producer system, before its consumer) so a producer->consumer pair ordered in the same frame sees the events that frame, instead of only after the next update_events swap.

### KF-25 Change-detection query surface

- **Status:** active. **Physics needs it:** no. **Kind:** capability.
- **Adds to the kernel:** A public per-archetype changed-tick column accessor usable from exclusive systems (transform propagation's dirty seed), and `any_changed_since` over a `ComponentMask` that skips non-intersecting archetypes. Both are speed-side, not storage.
- **Crates:** boyko_ecs, boyko_scene, boyko_ui
- **Plan:** None.
- **Merged from:** physics-scene-math `KF-exclusive-change-scan`; ui-input `KF5-changed-since-over-mask`; ui-lane `KF-25 Change-detection query surface`
- **Rows (3):** crates/boyko_scene/src/propagation.rs:124,404; [ui-lane] crates/boyko_ui/src/binding/bind_system.rs:38
- **Evidence:**
  - boyko_scene/src/propagation.rs:40 "per-archetype changed-tick-column accessor on the kernel (`read_changed_tick`"
  - boyko_ecs/src/ecs/core/ecs_master/component_api.rs:403 "pub fn any_changed_since(&self, ids: &[ComponentId], last_run: Tick, this_run: Tick) -> bool {"
  - boyko_ecs/src/ecs/core/component/component_mask.rs:8 "pub struct ComponentMask {"
  - D:/wt/ui:crates/boyko_ecs/src/ecs/core/ecs_master/component_api.rs:403 "pub fn any_changed_since(&self, ids: &[ComponentId], last_run: Tick, this_run: Tick) -> bool {"
  - D:/wt/ui:crates/boyko_ecs/src/ecs/core/component/component_mask.rs:8 "pub struct ComponentMask {"
- **Group notes:**
  - (physics-scene-math) Perf, not storage: the dirty seed stays a system-scratch column either way.
  - (ui-input) Performance side-effect not measured.
  - (ui-lane, note) Performance side-effect not measured.
  - (ui-lane, capability) any_changed_since taking a ComponentMask (the kernel id-set type) and skipping archetypes whose mask does not intersect it.

### KF-26 Kernel name table

- **Status:** active. **Physics needs it:** no. **Kind:** capability. **Engine design:** EK20.
- **Rev 2:** The engine design's EK20, one StrInterner (main:docs/unification/ENGINE-RUNTIME-ECS-DESIGN.md:915 "\| EK20 \| One StrInterner \| — \| services KF6 \| `Name`, tags, log \| here \|"). The reflect lane routes its planned C11 Str accessor here or to KF-03.
- **Adds to the kernel:** One process-global interned-name table owned by the kernel on kernel columns: a byte column, a span column and open-addressing slots. `&'static str` resolution is justified by in-place growth with no truncation. It replaces per-crate interners. Cold path, lowest priority.
- **Crates:** boyko_ecs, boyko_scene; other interners not examined
- **Plan:** None.
- **Merged from:** physics-scene-math `KF-kernel-name-table`; reflect-lane `Kernel name table / owned span on kernel columns`
- **Rows (4):** crates/boyko_scene/src/identity.rs:72,74,81,121
- **Evidence:**
  - boyko_scene/src/identity.rs:81 "static INTERNER: OnceLock&lt;Mutex&lt;InternerState>> = OnceLock::new();"
  - boyko_scene/src/identity.rs:121 "let leaked: &'static str = Box::leak(Box::&lt;str>::from(s));"
  - boyko_ecs/src/ecs/core/component/scratch/scratch_column.rs:33 "* **address-stable base** — `ComponentPool` grows IN PLACE (commits fresh"
  - D:/wt/reflect:docs/REFLECTION-PLAN-CORE.md:3800 "as *mut String, s.to_owned())`** on the original arena `*mut` provenance"
- **Group notes:**
  - (physics-scene-math) Cold, setup-time; lowest priority. Listed because a per-crate interner is exactly the crate-local wrapper principle 0 forbids.
  - (reflect-lane, note) C11's set_str performs one std-heap allocation per write into a `String` field. The plan's own census finds zero String fields in engine components (REFLECTION-PLAN-CORE.md:3806-3807). Decided: the Str accessor targets the engine's text carriers (inline bytes like UiName, or a KF-26 interned name id / KF-03 owned span), not std String. OVERTURNED only if a shipping component with a String field appears.
  - (reflect-lane, status) existing ledger features - the destination for PLANNED CORE rung C11 (Str), no row today

### KF-27 Query into kernel column

- **Status:** active. **Physics needs it:** no. **Kind:** API.
- **Adds to the kernel:** An entity query that writes into a `ScratchBuildView<Entity>` and keeps its matched-archetype list in a caller-held `QueryState` (the kernel's cached, generation-tracked match). This removes the caller's `arch_scratch` Vec and retires the Vec-returning `query_entities`.
- **Crates:** boyko_ecs, boyko_ui
- **Plan:** None.
- **Merged from:** ui-input `KF2-query-into-kernel-column`; ui-lane `KF-27 Query into kernel column`
- **Rows (12):** [ui-lane] crates/boyko_ui/src/binding/bind_system.rs:46,48,50; [ui-lane] crates/boyko_ui/src/interaction/focus.rs:131,133; [ui-lane] crates/boyko_ui/src/layout.rs:215; [ui-lane] crates/boyko_ui/src/resources.rs:254; [ui-lane] crates/boyko_ui/src/widgets.rs:73,75; [ui-lane] crates/boyko_ui/src/world/pick.rs:110,113,120
- **Evidence:**
  - boyko_ecs/src/ecs/core/ecs_master/entity_query_api.rs:108 "pub fn query_entities_buf("
  - boyko_ecs/src/ecs/core/iters/query_state.rs:128 "pub fn with_component_ids(includes: &[ComponentId]) -> Self {"
  - boyko_ecs/src/ecs/core/iters/query_state.rs:156 "pub fn iter_pre_terms&lt;'a>(&'a mut self, master: &'a ArchetypeMaster) -> QueryStateIter&lt;'a> {"
  - D:/wt/ui:crates/boyko_ecs/src/ecs/core/ecs_master/entity_query_api.rs:108 "pub fn query_entities_buf("
  - D:/wt/ui:crates/boyko_ecs/src/ecs/core/iters/query_state.rs:128 "pub fn with_component_ids(includes: &[ComponentId]) -> Self {"
  - D:/wt/ui:crates/boyko_ecs/src/ecs/core/iters/query_state.rs:156 "pub fn iter_pre_terms&lt;'a>(&'a mut self, master: &'a ArchetypeMaster) -> QueryStateIter&lt;'a> {"
- **Group notes:**
  - (ui-input) The four arch_ids rows become kernel-internal (QueryState::matched_ids, kernel group), so boyko_ui stops owning archetype bookkeeping. The Vec-returning query_entities (boyko_ecs/src/ecs/core/ecs_master/entity_query_api.rs:92 "pub fn query_entities(&self, component_ids: &[ComponentId]) -> Vec&lt;Entity> {") should go with it.
  - (ui-lane, note) The four arch_ids rows become kernel-internal (QueryState::matched_ids, kernel group), so boyko_ui stops owning archetype bookkeeping. The Vec-returning query_entities (D:/wt/ui:crates/boyko_ecs/src/ecs/core/ecs_master/entity_query_api.rs:92 "pub fn query_entities(&self, component_ids: &[ComponentId]) -> Vec&lt;Entity> {") should go with it.
  - (ui-lane, capability) An entity query that writes its result into a ScratchBuildView&lt;Entity> and keeps its matched-archetype list in a caller-held QueryState (the kernel's cached, generation-tracked archetype match), instead of `&mut Vec<Entity>` + a caller `&mut Vec<ArchetypeId>` re-derived on every call.

### KF-28 World read beside Commands

- **Status:** active. **Physics needs it:** no. **Kind:** capability.
- **Adds to the kernel:** A system shape that reads arbitrary live components and relations (read-only, dynamic by ComponentId) while holding ResMut + Commands. This deletes the `UiTreeView` parallel copy.
- **Crates:** boyko_ecs, boyko_ui
- **Plan:** None.
- **Merged from:** ui-input `KF7-world-read-beside-commands`; ui-lane `KF-28 World read beside Commands`
- **Rows (6):** [ui-lane] crates/boyko_ui/src/reload/reconcile.rs:332; [ui-lane] crates/boyko_ui/src/reload/tree_view.rs:40,81,89,100,107
- **Evidence:**
  - boyko_ui/src/reload/system.rs:132 "/// FunctionSystem cannot supply both a `ResMut` and read arbitrary live"
  - D:/wt/ui:crates/boyko_ui/src/reload/system.rs:132 "/// FunctionSystem cannot supply both a `ResMut` and read arbitrary live"
- **Group notes:**
  - (ui-input) Deletes the UiTreeView parallel copy (rows boyko_ui/src/reload/tree_view.rs:55/99/107/118/125 and row boyko_ui/src/reload/reconcile.rs:356). Alternatively an exclusive system with a kernel command queue; either way it is a kernel capability, not a boyko_ui wrapper.
  - (ui-lane, note) Deletes the UiTreeView parallel copy (rows boyko_ui/src/reload/tree_view.rs:55/99/107/118/125 and row boyko_ui/src/reload/reconcile.rs:356). Alternatively an exclusive system with a kernel command queue; either way it is a kernel capability, not a boyko_ui wrapper. \| ui-lane: the parallel copy is a defect generator, not only a copy - S6 had to widen LiveNode by three fields, and the lane's own comment records what happens when one is missed (D:/wt/ui:crates/boyko_ui/src/reload/tree_view.rs:60 "but here is dead code that silently drops on every round trip and goes"). The same kernel gap pushed the tween sink's insert into an on_add hook (D:/wt/ui:crates/boyko_ui/src/animation.rs:500 "helper that could both read and insert is `pub(crate)`."), so KF-28 has a second user in the lane.
  - (ui-lane, capability) A system shape that reads arbitrary live components and relations (read-only, dynamic by ComponentId) while holding ResMut + Commands, so a diff/serialize pass reads the ECS in place instead of snapshotting it.

### KF-29 Startup schedule

- **Status:** active. **Physics needs it:** no. **Kind:** capability. **Engine design:** EK9.
- **Rev 2:** The engine design's EK9 (main:docs/unification/ENGINE-RUNTIME-ECS-DESIGN.md:901 "\| EK9 \| Ordered startup stages \| startup `ScheduleBuilder` \| KF-startup-schedule; host K4 \| boot \| here \|").
- **Adds to the kernel:** A one-shot Startup schedule on the existing ScheduleBuilder. `add_startup_system` routes into it instead of into boxed closures in an App list.
- **Crates:** boyko_ecs, boyko_app, demos
- **Plan:** None.
- **Merged from:** ecs-services `KF-startup-schedule`
- **Rows (2):** crates/boyko_ecs/src/ecs/core/app/app.rs:175,514
- **Evidence:**
  - crates/boyko_ecs/src/ecs/core/app/app.rs:514 "self.startup.push(Box::new(move \|world: &mut EcsMaster\| {"
  - crates/boyko_ecs/src/ecs/core/app/app.rs:626 "let startup = std::mem::take(&mut self.startup);"
- **Group notes:**
  - (ecs-services) Storage then belongs to the ecs-schedule group's system rows.

### KF-30 App runner as fn pointer

- **Status:** active. **Physics needs it:** no. **Kind:** API.
- **Adds to the kernel:** `App::set_runner` takes a plain `fn(&mut App) -> AppExit`. The runner's state (WindowDesc) becomes a World Resource.
- **Crates:** boyko_ecs, boyko_app
- **Plan:** None.
- **Merged from:** app-demo `KF-app-runner-fn`
- **Rows (1):** crates/boyko_app/src/plugins.rs:803
- **Evidence:**
  - crates/boyko_ecs/src/ecs/core/app/app.rs:536 "pub fn set_runner(&mut self, runner: Box&lt;dyn FnOnce(&mut App) -> AppExit>) {"
  - crates/boyko_app/src/runner.rs:94 "pub(crate) title: &'static str,"

### KF-31 App owns the pool; PoolInner in one reservation

- **Status:** active. **Physics needs it:** no. **Kind:** layout + signature change.
- **Adds to the kernel:** The App owns `ThreadPool` by value, and `Schedule::run` / `ScheduleBuilder::build` take `&ThreadPool`. `PoolInner` lives in one address-stable VmReservation with its per-worker tables as inline MAX_WORKERS arrays. The boot handshake, the `Arc<[..]>` tables and `Arc<ThreadPool>` are deleted.
- **Crates:** boyko_threadpool, boyko_ecs, boyko_app, boyko_demo
- **Plan:** ALLOCATOR-DESIGN-SPACE decides "`App` owns the pool by value". Its TableSet is not needed (plan conflict).
- **Merged from:** app-demo `KF-app-owns-pool`; pool-utils-log `KF-pool-single-reservation`
- **Rows (19):** crates/boyko_threadpool/src/thread_pool.rs:128,132,135,401,412,529,664,672,673,688,709,711,713,714,756; crates/boyko_threadpool/src/worker.rs:43; crates/boyko_demo/src/app.rs:183,266; crates/boyko_demo/src/sim/runner.rs:153
- **Evidence:**
  - crates/boyko_ecs/src/ecs/core/app/app.rs:214 "pub fn with_pool(pool: Arc&lt;ThreadPool>) -> Self {"
  - docs/memory/ALLOCATOR-DESIGN-SPACE.md:331 (main checkout D:/claude/BoykoEngine, untracked draft) "\| `Arc<ThreadPool>` in `Schedule`, `ScheduleBuilder`, `App` (`schedule.rs:122`, `app.rs:191`) \| **removed**: `Schedule::run(&mut self, master, pool: &ThreadPool)`; `ScheduleBuilder::build(self, pool: &ThreadPool)`. `App` owns the pool by value \|"
  - [main checkout D:/claude/BoykoEngine, untracked rev-1 file] docs/memory/ALLOCATOR-DESIGN-SPACE.md:356 "one `TableSet` per pool ("PoolTables"): workers, stealers, lane rings, `ScopeArena`s, `PoolInner`; workers hold `NonNull<PoolInner>` valid until `Drop` joins them"; [main checkout D:/claude/BoykoEngine, untracked rev-1 file] docs/memory/ALLOCATOR-DESIGN-SPACE.md:352 "`App` owns the pool by value"
- **Group notes:**
  - (pool-utils-log) A kernel-internal re-layout rather than a new capability for other crates; listed because it changes public signatures. The plan's TableSet is NOT needed for it (plain inline arrays suffice for &lt;= 64 workers) - see plan_conflicts.

### KF-32 Memory library below the pool

- **Status:** active. **Physics needs it:** no. **Kind:** layering.
- **Adds to the kernel:** VmReservation reachable from `boyko_threadpool` without a cycle: the memory library's primitives move into a bottom crate that `boyko_ecs` re-exports, so the scheduler substrate allocates from the SAME library as the ECS storage. The alternative (fold the pool into `boyko_ecs`) is recorded but not chosen.
- **Crates:** new bottom crate, boyko_ecs, boyko_threadpool
- **Plan:** ALLOCATOR-DESIGN-SPACE `boyko_memory`.
- **Merged from:** pool-utils-log `KF-memory-below-the-pool`
- **Rows (9):** crates/boyko_threadpool/src/block.rs:482; crates/boyko_threadpool/src/thread_pool.rs:113,132,135,401,664,682; crates/boyko_threadpool/src/worker.rs:43,347
- **Evidence:**
  - [main checkout D:/claude/BoykoEngine, untracked rev-1 file] docs/memory/ALLOCATOR-DESIGN-SPACE.md:105 "`boyko_memory` — new crate containing `vm.rs`, `vm_column.rs`, `utils.rs`"; [main checkout D:/claude/BoykoEngine, untracked rev-1 file] docs/memory/ALLOCATOR-DESIGN-SPACE.md:105 "Reason: `boyko_threadpool` and `boyko_utils` need reservations and cannot depend on `boyko_ecs`"
- **Group notes:**
  - (pool-utils-log) NARROWED against the inventory, which listed 'every non-C, non-T row of this group': boyko_utils needs no edge (SparseMap's users are all in boyko_ecs, SparseSlotMap is deleted in favour of a kernel table), and boyko_log needs none (its rows go to DspBuf in the same crate or are std-owned). Only the pool needs it, and only VmReservation, not VmColumn. Alternative recorded, not chosen: fold boyko_threadpool into boyko_ecs (no cycle: crates/boyko_ecs/Cargo.toml:8 "boyko-threadpool = { path = "../boyko_threadpool" }" is the only edge between them) - costs the pool's standalone loom/Miri harness.

### KF-33 Scope arena on the memory library

- **Status:** active. **Physics needs it:** no. **Kind:** kernel-internal.
- **Adds to the kernel:** `ScopeBlock` backed by one VmReservation per THREAD identity, with a mark at scope entry and a rewind at the join. It replaces the std::alloc chunks and the chunk table. Thread identity comes from a const-initialised, Drop-free TLS pointer.
- **Crates:** boyko_threadpool, the bottom memory crate
- **Plan:** ALLOCATOR-DESIGN-SPACE ScopeArena, with the C2 identity fix and the W1 Miri RED-first obligation.
- **Merged from:** pool-utils-log `KF-scope-arena-vm`
- **Rows (4):** crates/boyko_threadpool/src/block.rs:482; crates/boyko_threadpool/src/scope.rs:1096; crates/boyko_threadpool/src/thread_pool.rs:278,328
- **Evidence:**
  - [main checkout D:/claude/BoykoEngine, untracked rev-1 file] docs/memory/ALLOCATOR-DESIGN-SPACE.md:113 "\| **Scope** \| `ScopeArena` (one per thread slot, `W+1`), replaces `ScopeBlock`'s `std::alloc` chunks and `Box<ScopeShared>`"; [main checkout D:/claude/BoykoEngine, untracked rev-1 file] docs/memory/ALLOCATOR-DESIGN-SPACE.md:326 "chunks = sub-ranges of the same `ScopeArena`"; fix [main checkout D:/claude/BoykoEngine, untracked rev-1 file] docs/memory/ALLOCATOR-DESIGN-SPACE.md:461 "\| C2 \| **`ScopeArena` slots keyed by `wid` collide.**"; obligation [main checkout D:/claude/BoykoEngine, untracked rev-1 file] docs/memory/ALLOCATOR-DESIGN-SPACE.md:467 "\| W1 \| The KE16 completion-protector Miri gate keys on a DEALLOCATION that rung 1a removes"
- **Group notes:**
  - (pool-utils-log) Thread identity via a const-initialised, Drop-free TLS pointer (no std TLS destructor), set by worker_main from the pool's per-worker table and, for external installers, by a spare-slot claim at the outermost install released in InstallGuard.

### KF-34 In-house work-stealing lanes

- **Status:** active. **Physics needs it:** no. **Kind:** kernel-internal.
- **Adds to the kernel:** In-house fixed-capacity Chase-Lev rings per lane and a bounded MPMC injector ring in the pool's VM reservation. This is the only route that removes crossbeam-epoch's per-thread Local and std's System-allocated TLS destructor entry, which a `#[global_allocator]` gate cannot see.
- **Crates:** boyko_threadpool
- **Plan:** ALLOCATOR-DESIGN-SPACE rung 1d (loom model mandatory).
- **Merged from:** pool-utils-log `KF-inhouse-lanes`
- **Rows (4):** crates/boyko_threadpool/src/thread_pool.rs:113,132,682; crates/boyko_threadpool/src/worker.rs:347
- **Evidence:**
  - [main checkout D:/claude/BoykoEngine, untracked rev-1 file] docs/memory/ALLOCATOR-DESIGN-SPACE.md:328 "**in-house bounded Chase-Lev** per lane on a `Table<Task>` ring in the pool's `TableSet`"
- **Group notes:**
  - (pool-utils-log) Replaces a third-party crate; the plan lists it as rung 1d. Loom model mandatory per the plan.

### KF-35 VmColumn ensure_len_zeroed

- **Status:** active. **Physics needs it:** no. **Kind:** API.
- **Adds to the kernel:** VmColumn grows its length into committed-zero pages without writing, so an index-keyed sparse column whose absent encoding is 0 grows by commit alone.
- **Crates:** the memory library
- **Plan:** ALLOCATOR-DESIGN-SPACE `ensure_len_zeroed`.
- **Merged from:** pool-utils-log `KF-vmcolumn-ensure-len-zeroed`
- **Rows (1):** crates/boyko_utils/src/sparse_map/sparse_map.rs:7
- **Evidence:**
  - [main checkout D:/claude/BoykoEngine, untracked rev-1 file] docs/memory/ALLOCATOR-DESIGN-SPACE.md:406 "add `ensure_len_zeroed`, `pop`"
- **Group notes:**
  - (pool-utils-log) The ecs-storage group recorded the same need for component_pool_bundle's SparseMap ('VmColumn&lt;u32> needs a fill-on-grow op'); with the index+1 encoding a fill is never needed, only the zeroed grow.

### KF-36 Kernel storage reachable from the RHI (+ generational table)

- **Status:** decided. **Physics needs it:** no. **Kind:** layering (decided in rev 2).
- **Rev 2:** Route decided (section Decisions): (a), an rhi -> boyko_ecs edge, so the RHI owns kernel columns and runs as NonSend systems. Route (b), a leaf storage crate, would give the RHI VmColumns but not ECS forms. Performance is equal (the same columns either way); ARCHITECTURE.md's "the RHI does not depend on boyko_ecs" is amended.
- **Adds to the kernel:** boyko_rhi and boyko_rhi_vulkan own kernel columns and run as NonSend systems. The RHI registries become kernel generational tables with the Assets&lt;T> shape. Route (a) is an rhi -> boyko_ecs edge, which makes no cycle today. Route (b) is a leaf storage crate. The host present/record path moves from the runner into NonSend systems.
- **Crates:** boyko_ecs, boyko_rhi, boyko_rhi_vulkan, boyko_app, boyko_utils (SparseSlotMap deleted)
- **Plan:** Conflict: boyko_rhi/src/lib.rs plans the ecs edge, while ARCHITECTURE.md says the RHI does not depend on boyko_ecs.
- **Merged from:** rhi `KF-kernel-storage-below-the-rhi`; pool-utils-log `KF-generational-table-reachable-from-rhi`
- **Rows (44):** crates/boyko_utils/src/sparse_map/sparse_slot_map.rs:41,42,44; crates/boyko_rhi/src/handle.rs:78; crates/boyko_rhi_vulkan/src/device.rs:677,688,698,913,2358,2413,2545,2721,3114; crates/boyko_rhi_vulkan/src/framegraph/graph.rs:126,127,136,145,148,156,157,158,161,162,163,164,165,172,188,200,203,204,205; crates/boyko_rhi_vulkan/src/memory.rs:700; crates/boyko_rhi_vulkan/src/present/frame_driver.rs:48; crates/boyko_rhi_vulkan/src/present/surface.rs:180,238; crates/boyko_rhi_vulkan/src/present/swapchain.rs:64,66; crates/boyko_rhi_vulkan/src/suballocator.rs:65,68; crates/boyko_rhi_vulkan/src/window.rs:122,230,365,761
- **Evidence:**
  - The layering rule docs/ARCHITECTURE.md:174 "itself does NOT depend on `boyko_ecs` (so no cycle)." and the substrate choice boyko_rhi_vulkan/src/framegraph/graph.rs:13 "`boyko_ecs`'s `pub(crate)` `VmReservation` public; a single-reservation" are why these rows are std::Vec today. It is an architecture fork, not a values call, and it precedes every kernel destination in this group. It is also where the host migration lands: the present/record path must move from the runner (boyko_app/src/runner.rs:2640 "host.renderer.render_gbuffer_frame(") into NonSend systems for the FrameGraph and the singleton rows to reach their forms.
  - crates/boyko_rhi/src/lib.rs:28 "The `boyko_ecs` dependency and core's" / crates/boyko_rhi/src/lib.rs:29 "`DeviceColumnHandle(u64)` newtype land in Phase 4; for now the registry uses" (planned the edge) vs docs/ARCHITECTURE.md:174 "itself does NOT depend on `boyko_ecs` (so no cycle)."
- **Group notes:**
  - (pool-utils-log) Same feature as the rhi group's 'KF-2 kernel storage reachable from the RHI layer' (its build script, ledger/ecsform_rhi/build.py) - dedupe on merge. Two realisations: the boyko_rhi -> boyko_ecs edge its header planned (no cycle: boyko_ecs does not depend on boyko_rhi), or the generational core moved into the bottom memory crate. The choice is the owner's (layering).

### KF-37 Assets&lt;T>::adopt_retiring

- **Status:** superseded. **Physics needs it:** no. **Kind:** API. **Engine design:** K6' (engine).
- **Rev 2:** SUPERSEDED in rev 2 by engine decision Q1 (assets are entities): asset values become K3 group columns, and a retired value is released through the stamped-horizon group release K6' (ledger KF-49 in rev 3) (engine design rev 3: main:docs/unification/ENGINE-RUNTIME-ECS-DESIGN.md:2559 "> \| K6′ **(rev 3, re-filed)** \| Stamped horizon on `release_dying` \|"). The live fill-reject leak is fixed NOW with the existing Orphaned*Gpu queues (defect B).
- **Adds to the kernel:** A fill-rejected value becomes a Retiring row of its own store and is freed behind the same fence gate. This also closes the live device-memory leak on fill-reject (section (f), render findings).
- **Crates:** boyko_ecs, boyko_render, boyko_scene
- **Plan:** None.
- **Merged from:** render `KF-assets-adopt-retiring`
- **Rows that named it before rev 2 (2; none needs it now):** crates/boyko_render/src/mesh_assets.rs:712; crates/boyko_render/src/texture.rs:928
- **Evidence:**
  - boyko_ecs/src/ecs/core/asset/assets.rs:453 "pub fn fill(&mut self, handle: Handle&lt;T>, value: T) -> Result&lt;(), (AssetError, T)> {"
  - boyko_ecs/src/ecs/core/asset/assets.rs:998 "pub fn retire(&mut self, slot: u32) -> Option&lt;T> {"
  - boyko_render/src/gpu_upload.rs:120 "let _ = assets.fill(staged.handle, gpu);"
  - boyko_render/src/mesh.rs:204 "trivial drop), freeing NO device memory."
- **Group notes:**
  - (render) It also closes the live defect this pass found: upload_assets drops a rejected MeshGpu/TextureGpu, whose drop glue frees no device memory. It would also unblock the recorded bindless-slot = asset-row-index unification.

### KF-38 Assets&lt;T>::iter_mut + owned drain

- **Status:** withdrawn. **Physics needs it:** no. **Kind:** API.
- **Rev 2:** WITHDRAWN in rev 2: under engine Q1 the asset values are K3 group columns iterated as groups; the engine design deleted its own `Assets<T>` dedup feature EK17 (main:docs/unification/ENGINE-RUNTIME-ECS-DESIGN.md:2553 "> \| ~~EK17~~ \| **deleted**: Q1 = (a) is decided \| — \| — \|").
- **Adds to the kernel:** Whole-table mutable iteration and an owned drain, so passes stop collecting handle lists first.
- **Crates:** boyko_ecs, boyko_render
- **Plan:** None.
- **Merged from:** render `KF-assets-iter-mut`
- **Rows that named it before rev 2 (3; none needs it now):** crates/boyko_render/src/gpu_upload.rs:216; crates/boyko_render/src/mesh_assets.rs:582; crates/boyko_render/src/texture.rs:709
- **Evidence:**
  - boyko_ecs/src/ecs/core/asset/assets.rs:703 "pub fn iter(&self) -> impl Iterator&lt;Item = (Handle&lt;T>, &T)> + '_ {"
  - boyko_render/src/mesh_assets.rs:577 "// `Assets<T>` exposes no owned/mutable whole-table iteration (only the"

### KF-39 ComponentPool add inert row

- **Status:** active. **Physics needs it:** no. **Kind:** API.
- **Adds to the kernel:** Appends one zeroed, not-live row without a source buffer. This is legal on a pool with drop glue, because the occupancy bitmap gates drops.
- **Crates:** boyko_ecs
- **Plan:** None.
- **Merged from:** ecs-services `KF-pool-add-inert-row`
- **Rows (1):** crates/boyko_ecs/src/ecs/core/asset/assets.rs:383
- **Evidence:**
  - crates/boyko_ecs/src/ecs/memory/component_pool.rs:846 "pub fn add(&mut self, component_bytes: &[u8]) -> Option&lt;usize> {"
  - crates/boyko_ecs/src/ecs/memory/component_pool.rs:1031 "self.drop_fn.is_none(),"
- **Group notes:**
  - (ecs-services) An API addition, not a primitive. Replaces the inventory's 'ComponentPool::add_uninit'.

### KF-40 Device column residency (existing seam)

- **Status:** active. **Physics needs it:** no. **Kind:** existing seam. **Engine design:** EK16.
- **Rev 2:** The engine design's EK16, kernel half only (main:docs/unification/ENGINE-RUNTIME-ECS-DESIGN.md:910 "\| EK16 **(rev 2)** \| Device-column meta fold (audit Stage 5), kernel only \|").
- **Adds to the kernel:** The GPU residency record of a column lives on the `PoolBacking::Device` arm and is resolved through the archetype's pool bundle. The render-side pair table is deleted.
- **Crates:** boyko_ecs, boyko_render
- **Plan:** ARCH-AUDIT-ECS-DATA-REMEDIATION Stage 5.
- **Merged from:** render `KF-device-column-residency (existing seam, ARCH-AUDIT Stage 5)`
- **Rows (2):** crates/boyko_render/src/gpu_column.rs:532,563
- **Evidence:**
  - boyko_ecs/src/ecs/memory/component_pool.rs:93 "Device(Box&lt;DeviceColumn>),"
  - boyko_ecs/src/ecs/memory/device_column.rs:33 "/// Holds the opaque [`DeviceColumnHandle`] plus the device-side live/committed"
  - D:/wt/joltab/docs/ARCH-AUDIT-ECS-DATA-REMEDIATION.md:31 "- **Stage 5 (S4):** fold `GpuColumnManager.meta` into per-column archetype metadata (`PoolBacking::Device` arm). Cold; lowest urgency."
- **Group notes:**
  - (render) Not new: a planned fill of an existing kernel seam, listed so the design sees it.

### KF-41 Intrusive free lists

- **Status:** decided. **Physics needs it:** no. **Kind:** kernel-internal technique.
- **Rev 2:** Decided (section Decisions): intrusive dead-slot lists wherever the dead slot has a free word (InlandStore, the DenseStore s2e tombstone, archetype slab slots, registry records); a VmColumn stack where a slot must hold DEAD bytes (the physics K3 `dying` list, design section 8). LIFO order is preserved in both.
- **Adds to the kernel:** A storage's LIFO free list is threaded through its own dead slots (InlandStore, DenseStore s2e tombstones, archetype slab slots, registry records). Reuse order is preserved exactly, and the separate container is deleted.
- **Crates:** boyko_ecs
- **Plan:** RUST-ERGONOMICS sites it. Three plans disagree for free_entity_ids (plan conflicts PC1/PC2).
- **Merged from:** ecs-storage `KF-intrusive-free-lists`
- **Rows (4):** crates/boyko_ecs/src/ecs/core/archetype/archetype_bundle.rs:135; crates/boyko_ecs/src/ecs/core/archetype/archetype_registry.rs:11; crates/boyko_ecs/src/ecs/core/component/dense/dense_store.rs:126; crates/boyko_ecs/src/ecs/core/entity/entity_master.rs:73
- **Evidence:**
  - D:/wt/joltab/docs/RUST-ERGONOMICS.md:759 "`free_entity_ids: Vec<EntityId>` — a parallel data system by"
  - D:/wt/joltab/docs/RUST-ERGONOMICS.md:760 "principle 0 — can live in the dead slots' own bytes, halving the reallocations of a despawn"
  - crates/boyko_ecs/src/ecs/core/component/dense/dense_store.rs:34 "pub(crate) const TOMBSTONE: EntityId = EntityId(usize::MAX);"
  - crates/boyko_ecs/src/ecs/core/component/dense/dense_store.rs:124 "/// LIFO free list of tombstoned slots. `insert` pops here first so a freed"
  - crates/boyko_ecs/src/ecs/core/entity/entity_master.rs:378 "self.free_entity_ids.push(entity_id);"

### KF-42 Atomic views over kernel columns

- **Status:** active. **Physics needs it:** no. **Kind:** kernel-internal technique.
- **Adds to the kernel:** Rows are stored as plain u64 in VmColumn/ComponentPool and accessed concurrently through `AtomicU64::from_ptr` views. The column shape changes only under `&mut`. This answers the design's own W3 finding.
- **Crates:** boyko_ecs (enable-state kernel)
- **Plan:** ALLOCATOR-DESIGN-SPACE W3 (refuted there, answered here).
- **Merged from:** ecs-storage `KF-atomic-view-over-column`
- **Rows (4):** crates/boyko_ecs/src/ecs/core/component/enable/enable_presence.rs:318; crates/boyko_ecs/src/ecs/core/component/enable/enable_store.rs:72,163,173
- **Evidence:**
  - crates/boyko_ecs/src/ecs/core/component/enable/enable_store.rs:60 "pub(crate) struct EnablePage([AtomicU64; WORDS_PER_PAGE]);"
  - D:/claude/BoykoEngine/docs/memory/ALLOCATOR-DESIGN-SPACE.md:469 "\| W3 \| `VmColumn<EnablePage>` cannot type-check"
  - crates/boyko_ecs/src/ecs/memory/vm_column.rs:30 "is deliberately NOT a general `Vec` replacement for droppable `T`."

### KF-43 Static type descriptors (derive output allocation-free)

- **Status:** active. **Physics needs it:** no. **Kind:** kernel-internal contract.
- **Rev 2:** The reflect lane REALISES this contract for the reflect descriptor (per-type statics installed through a static OnceLock table), with no row.
- **Adds to the kernel:** Every derive-generated `&'static` type descriptor resolves with no allocator: a per-type static sized at expansion and initialised once through OnceLock, or the literal `&[]`. First use of a type after steady state is then allocation-free.
- **Crates:** boyko_macros, boyko_ecs (the hand-written twin self_bundle.rs), every deriving crate
- **Plan:** Conflicts with archived SBC8 (a Box::leak per Bundle type), which live code comments still cite.
- **Merged from:** macros-aether `static-type-descriptors`; reflect-lane `Static type descriptors (derive output allocation-free)`
- **Rows (5):** crates/boyko_macros/src/bundle.rs:349; crates/boyko_macros/src/component.rs:900; crates/boyko_macros/src/event.rs:378,383,413
- **Evidence:**
  - D:/wt/reflect:crates/boyko_macros/src/reflect.rs:834 "static __REFLECT_FIELDS: [::boyko_reflect::FieldInfo; #field_count] = ["
  - D:/wt/reflect:crates/boyko_macros/src/reflect.rs:838 "static __REFLECT_TYPE_INFO: ::boyko_reflect::TypeInfo ="
  - D:/wt/reflect:crates/boyko_reflect/src/registry.rs:29 "static REFLECT: [OnceLock&lt;&'static TypeInfo>; MAX_COMPONENTS] ="
- **Group notes:**
  - (macros-aether) The kernel also leaks once-per-type / once-per-key descriptors whose LENGTH is known only at runtime, so a per-type static cannot absorb them; they are ecs-group rows and are not decided here: crates/boyko_ecs/src/ecs/core/bundle/bundle_column_cache.rs:335 "let pool_ids: &'static [InlandPoolId] = Box::leak(pool_ids_boxed);"; crates/boyko_ecs/src/ecs/core/bundle/bundle_column_cache.rs:454 "let required_missing: &'static [RequiredEntry] = Box::leak(missing.into_boxed_slice());"; crates/boyko_ecs/src/ecs/core/iters/component_set.rs:59 "TUPLE_SLICES[idx].get_or_init(\|\| Box::leak(init().into_boxed_slice()))"; crates/boyko_ecs/src/ecs/core/iters/component_set.rs:127 "SINGLE_COMPONENT_CACHE[id.0].get_or_init(\|\| Box::leak(vec![id].into_boxed_slice()))"; crates/boyko_ecs/src/ecs/core/component/component_registry/flags.rs:114 "let leaked: &'static [FlagDirectEntry] = Box::leak(builder.into_entries());"; crates/boyko_ecs/src/ecs/core/component/component_registry/required.rs:338 "let leaked_entries: &'static [RequiredEntry] = Box::leak(out.into_boxed_slice());"; crates/boyko_ecs/src/ecs/core/component/component_registry/tags.rs:192 "let leaked: &'static str = Box::leak(Box::&lt;str>::from(name));"; and a per-world cell crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs:960 "NonNull::from(Box::leak(cell));". If they need storage, the ECS form is still kernel-internal (the registry's own storage on the memory library), not a primitive standing beside ComponentPool.
  - (reflect-lane, note) The #[component(reflect)] emission is exactly KF-43's contract: a per-type static sized at expansion, installed once through a OnceLock slot of a static table, no allocator. The ledger's existing KF-43 row for the self-bundle leak (joltab component.rs:900, lane component.rs:968 "] = ::std::boxed::Box::leak(::std::boxed::Box::new([") is untouched by the lane and stays a row of macros-aether.
  - (reflect-lane, status) existing ledger feature - REALISED by the lane for the reflect descriptor; no row

### KF-44 Per-system state for exclusive systems

- **Status:** active. **Physics needs it:** no. **Kind:** capability. **Engine design:** EK2.
- **Rev 2:** Added by gap 7.
- **Adds to the kernel:** A `fn(&mut EcsMaster)` body gets a `Local<T>` and its `(last_run, this_run)` window, like a function system. Today an exclusive system has no per-param state, so 20 rows park per-system scratch in a world-global singleton Resource.
- **Crates:** boyko_ecs (provides); boyko_ui, boyko_scene
- **Plan:** = the engine design's EK2 (main:docs/unification/ENGINE-RUNTIME-ECS-DESIGN.md:894 "\| EK2 \| Exclusive system context \| `(last_run, this_run)` + `Local<S>` for exclusive bodies \|").
- **Merged from:** ui-input `KF44-exclusive-system-local`
- **Rows (21):** crates/boyko_scene/src/propagation.rs:120,124,133,404; [ui-lane] crates/boyko_ui/src/binding/bind_system.rs:46,48; [ui-lane] crates/boyko_ui/src/interaction/focus.rs:117,119,122,131,136; [ui-lane] crates/boyko_ui/src/resources.rs:216,218,226,234,239,247; [ui-lane] crates/boyko_ui/src/widgets.rs:73; [ui-lane] crates/boyko_ui/src/world/pick.rs:110,113,117
- **Evidence:**
  - joltab:crates/boyko_ecs/src/ecs/core/system/exclusive_function_system.rs:15 "//!    `SystemParamFunction`. There is no param tuple, no per-param state,"
  - joltab:crates/boyko_ecs/src/ecs/core/system/params/local.rs:62 "pub struct Local&lt;'s, T: Send + Sync + Default + 'static>(pub(crate) &'s mut T);"
  - main:docs/unification/ENGINE-RUNTIME-ECS-DESIGN.md:894 "\| EK2 \| Exclusive system context \| `(last_run, this_run)` + `Local<S>` for exclusive bodies \| UI KF-A; services KF3 \| `ui_bind_apply` \| here \|"
- **Group notes:**
  - (ui-input) rev2 GAP 7. 20 rows (ui 17, scene 3) park per-system scratch in a singleton Resource only because exclusive systems cannot own state.

### KF-45 Engine thread-context column (replaces every thread_local!)

- **Status:** active. **Physics needs it:** yes. **Kind:** capability (memory library, below the pool).
- **Rev 2:** Added by gap 1. **Correction (writer change W7):** gap 1 cited the physics decisions file for the explicit-context route ("RunCtx trampoline"). That row was superseded in place after the citation: main:docs/physics/PHYSICS-ECS-UNIFICATION-DECISIONS.md:89 "\| ~~RunCtx trampoline vs TLS-depth fallback (gang nesting rule)~~ \|". There is no run context on the task path, so the slot route is the general one. The performance comparison is unchanged. Overturned by: an A/B of `worker/body_1us_tasks_64W` on x86_64-pc-windows-msvc (native TLS) showing the slot route slower beyond its band.
- **Adds to the kernel:** One VmReservation-backed column of 64-B records, one per engine thread; every former `thread_local!` is a field. Reached by slot through the platform thread-control-block pointer (an id -> slot table), or by an explicit context parameter where a call chain already carries one. Removes 12 System-allocated os-key cells per thread on windows-gnu and two contended RMWs plus FlsSetValue per read.
- **Crates:** KF-32 memory layer (provides); boyko_threadpool, boyko_diag, boyko_log, boyko_ecs
- **Plan:** None; decided by gap 1 on the measured windows-gnu TLS cost (joltab:docs/threadpool/RUSTC-198-WINDOWS-GNU-TLS.md:1 "# rustc 1.98 on `x86_64-pc-windows-gnu`: every `thread_local!` read costs two contended RMWs and a Win32 call").
- **Merged from:** pool-utils-log `KF45-engine-thread-context`
- **Rows (12):** crates/boyko_ecs/src/ecs/core/component/component_registry/required.rs:153; crates/boyko_ecs/src/ecs/core/component/hooks/scope.rs:31; crates/boyko_ecs/src/ecs/core/component/observers/propagate.rs:30; crates/boyko_ecs/src/ecs/core/hierarchy/commands.rs:127; crates/boyko_ecs/src/ecs/core/relationship/mod.rs:90,152; crates/boyko_diag/src/lane.rs:139; crates/boyko_log/src/drain_owner.rs:42; crates/boyko_log/src/sync_out.rs:75; crates/boyko_threadpool/src/tls.rs:169,196,205
- **Evidence:**
  - rust-src(stable-x86_64-pc-windows-gnu):library/std/src/sys/thread_local/guard/windows.rs:116 "// `#[thread_local]` is unavailable on windows-gnu (`target_thread_local` is off),"
  - rust-src(stable-x86_64-pc-windows-gnu):library/std/src/sys/thread_local/os.rs:110 "let ptr: *mut Value&lt;T> = (unsafe { System.alloc(layout) }).cast();"
  - joltab:docs/threadpool/RUSTC-198-WINDOWS-GNU-TLS.md:1 "# rustc 1.98 on `x86_64-pc-windows-gnu`: every `thread_local!` read costs two contended RMWs and a Win32 call"
  - main:docs/physics/PHYSICS-ECS-UNIFICATION-DECISIONS.md:89 "\| ~~RunCtx trampoline vs TLS-depth fallback (gang nesting rule)~~ \| **Superseded by design rev 3** (P-§10.1): lanes are tickets on a per-pool `LaneBoard` that only `worker_main`'s top level probes. No `run_task` site carries a run context, so neither mechanism is built." [rev2 writer correction] The decisions-file row cited above was superseded in place after gap 1 cited it: the gang uses a per-pool LaneBoard and no run_task site carries a run context. So the RunCtx half of the KF-45 route does not exist; a static here takes the slot route (thread-control-block pointer -> id->slot table) unless its own call chain already passes an explicit context. The performance comparison against thread_local! is unchanged: one plain load plus an L1 probe against two contended RMWs plus FlsSetValue per read.
- **Group notes:**
  - (pool-utils-log) rev2 GAP 1. Replaces 12 thread_local! statics in shipped code (9 newly rowed + 3 already rowed).

### KF-46 Window entities

- **Status:** active. **Physics needs it:** no. **Kind:** host capability (+ dense address stability, existing). **Engine design:** ED18 (engine).
- **Rev 2:** Added by gap 8.
- **Adds to the kernel:** One entity per OS window; per-window data as components; OS-held pointers in a dense slot; raw input events carry the window Entity.
- **Crates:** boyko_app (provides); boyko_rhi_vulkan, boyko_input
- **Plan:** Engine decision Q3 (main:docs/unification/ENGINE-RUNTIME-ECS-DECISIONS.md:25 "\| Q3 \| Multiplicity in v1 \| **(b) Window and player are entities now** \| no - the design recommended (a) \|").
- **Merged from:** rhi `KF46-window-entities`
- **Rows (13):** crates/boyko_rhi_vulkan/src/present/frame_driver.rs:48; crates/boyko_rhi_vulkan/src/present/surface.rs:180,238; crates/boyko_rhi_vulkan/src/present/swapchain.rs:64,66; crates/boyko_rhi_vulkan/src/window.rs:122,230,274,352,365,761; crates/boyko_input/src/raw/queue.rs:35,59
- **Evidence:**
  - joltab:crates/boyko_app/src/window_info.rs:19 "pub struct WindowInfo {"
  - main:docs/unification/ENGINE-RUNTIME-ECS-DECISIONS.md:57 "- **Cost.** Both options cost the same per frame. Per-window and per-player data are touched once per"
  - main:docs/unification/ENGINE-RUNTIME-ECS-DECISIONS.md:58 "frame per instance, so a component on an entity costs nothing measurable over a singleton resource."
  - joltab:crates/boyko_rhi_vulkan/src/window.rs:372 "os::SetWindowLongPtrW(hwnd, os::GWLP_USERDATA, input_ring as isize);"
- **Group notes:**
  - (rhi) rev2 GAP 8. 13 window rows; 6 change form, 7 keep event / system-scratch with the window entity as owner.

### KF-47 By-id structural seam (structural ops by ComponentId + bytes)

- **Status:** active. **Physics needs it:** no. **Kind:** capability (shipped by the reflect lane).
- **Rev 2:** Added from the reflect lane (its KF-R1). Its only heap is the two SmallList4 spill rows it shares with every migration.
- **Adds to the kernel:** Attach / detach / mark-changed by ComponentId, with the bytes moved into the row, `#[require]` honoured, hooks and observers fired, and no heap in its own code (its scratch is two stack arrays). It is the ECS-native route for scene load, prefab instantiation (KF-13), serialize load (KF-08/KF-09), undo and network apply.
- **Crates:** boyko_ecs (provides, on feat/reflection); reflection, scene load, prefab, serialize, undo, network apply
- **Plan:** The reflect lane's EG2 (owner-approved as B.13 #2). D:/wt/reflect:crates/boyko_ecs/src/ecs/core/ecs_master/seam_by_id.rs:200 "pub fn add_component_by_id(".
- **Merged from:** reflect-lane `By-id structural seam (structural ops by ComponentId + bytes)`
- **Rows (2):** [reflect-lane] crates/boyko_ecs/src/ecs/core/commands/migration_helpers.rs:2171,2172
- **Evidence:**
  - D:/wt/reflect:crates/boyko_ecs/src/ecs/core/ecs_master/seam_by_id.rs:200 "pub fn add_component_by_id("
  - D:/wt/reflect:crates/boyko_ecs/src/ecs/core/ecs_master/seam_by_id.rs:500 "pub fn remove_component_by_id(&mut self, entity: Entity, id: ComponentId) -> bool {"
  - D:/wt/reflect:crates/boyko_ecs/src/ecs/core/ecs_master/seam_by_id.rs:616 "pub fn mark_component_changed(&mut self, entity: Entity, id: ComponentId) -> bool {"
  - D:/wt/reflect:crates/boyko_ecs/src/ecs/core/component/component_registry/tags.rs:225 "pub fn try_from_component_id(id: ComponentId) -> Option&lt;Self> {"
- **Group notes:**
  - (reflect-lane, note) A first-class kernel capability every crate can use the same way: attach / detach / mark-changed by ComponentId with the bytes MOVED into the row, #[require] honoured, hooks and observers fired, zero heap in its own code (its scratch is two stack arrays; its only heap is the two SmallList4 spill rows above, shared with every migration). It is the ECS-native route for every consumer that today stages typed values before a structural op - scene load, prefab instantiation (KF-13), serialize load (KF-08/KF-09), undo, network apply - candidates only, no ledger row counted as needing it.
  - (reflect-lane, status) NEW, provided and shipped by the lane in boyko_ecs (EG2, owner-approved B.13 #2)

### KF-48 Count-only relation (asset refcount)

- **Status:** active. **Physics needs it:** no. **Kind:** capability. **Engine design:** EK15b.
- **Rev 3:** Added in rev 3 (item 2, engine Q1).
- **Adds to the kernel:** A `CountOnly` relationship-target collection: add / remove are +1 / -1, the target cannot enumerate its sources, the target-despawn cascade is statically disabled, and despawning a target whose count is above 0 becomes retire-at-0.
- **Crates:** boyko_ecs (provides); boyko_render, boyko_scene
- **Plan:** = the engine design's EK15b (main:docs/unification/ENGINE-RUNTIME-ECS-DESIGN.md:908 "\| EK15b **(rev 2)** \| Count-only collection \| `CountOnly` impl: `add`/`remove` = ±1, no iteration; the").
- **Merged from:** the physics entity model (no group feature)
- **Rows (1):** crates/boyko_scene/src/asset_refs.rs:99

### KF-49 Deferred dense-group release with a horizon (K6')

- **Status:** active. **Physics needs it:** conditional. **Kind:** capability. **Physics design:** K6. **Engine design:** K6' (engine).
- **Rev 3:** Added in rev 3 (item 2, engine Q1). It is the ledger name for what superseded KF-37.
- **Adds to the kernel:** A K3 group with `RELEASE = Deferred` keeps a removed slot in its `dying` list with the bytes intact; `release_dense_group::<G>(horizon)` returns to `free` only the slots stamped before the horizon, with a visitor that frees the slot's device lane in the same pass.
- **Crates:** boyko_ecs (provides); boyko_render, boyko_scene; physics uses the horizon-free form
- **Plan:** The physics design's K6 generalised by the engine design (main:docs/unification/ENGINE-RUNTIME-ECS-DESIGN.md:589 "\| 14 **(rev 2)** \| orphan queues \| NonSend `Vec<(T,u64)>` \| **K3 deferred release with a horizon** (ED16)").
- **Merged from:** the physics entity model (no group feature)
- **Rows (4):** crates/boyko_scene/src/asset_refs.rs:149; crates/boyko_render/src/asset_refcount.rs:556; crates/boyko_render/src/mesh_assets.rs:712; crates/boyko_render/src/texture.rs:928

## Primitives refuted

Every memory primitive that an inventory or recount proposed beside ComponentPool, merged into families, with the ECS form that replaced it. Each group file carries that group's verdicts verbatim, with evidence. The two lanes proposed no new primitive.

| family | proposed as (group) | replaced by |
|---|---|---|
| Exported / id-free VmColumn and VmColumn extensions | pub VmColumn (render, rhi), id-free typed column (physics), ComponentId-free constructor (ui-input), VmColumn domain extension for non-granule sizes (codec-tools), grow_filled / grow_zeroed / pop (ecs-storage) | The existing public `ScratchColumn` (ComponentPool-backed, any layout) owned by a Resource or a system, plus KF-01. VmColumn stays kernel-private. |
| Non-Copy / drop-aware columns and boxes | VmSlab&lt;T> (ecs-storage, ecs-schedule), VmQueue&lt;T> (ecs-services, render), VmDropColumn&lt;T> (rhi), VmVec&lt;T> (pool-utils-log), PinnedCell / VmBox (rhi) | A ComponentPool with registered drop glue behind KF-02; a dense component (SystemBox); Retiring rows of the same `Assets<T>` (KF-37); a NonSend resource slot. |
| Bump / scratch arenas | FrameBump (app-demo, ecs-services), BootArena (app-demo), LoadArena (render, ui-input), CallScratch / ScratchVec (codec-tools), BuildArena and VmScratch (ecs-schedule), WorldArena, StaticArena, ScratchStack, ScratchList (ecs-storage), ScratchStack (ui-input), ScratchArena (rhi), ScratchOut (render) | system-scratch on ScratchColumn (KF-01); KF-05 for `&mut` kernel paths; enable-state (ScratchOut, via KF-17); elimination (stack arrays, in-place sorts, writing straight into mapped memory); compile-time statics. |
| Byte buffers | VmBytes (ecs-services, codec-tools), ByteSink (ui-input) | `ScratchColumn<u8>` / `VmColumn<MaybeUninit<u8>>` plus KF-06; event form for the command channel; resource-column payload lanes (KF-09). |
| Ragged / CSR / slices / list pools | RaggedColumn (physics-scene-math), VmJagged (ecs-schedule), CsrColumn (ui-input), VmSlice (render), SlabListPool (ecs-storage), SlabHeap and EntityListPool (ecs-services) | relation (KF-11); KF-03 owned span / CSR ranges on ScratchColumns; ComponentMask; intrusive per-pattern lists. |
| Strings, inline buffers, diagnostics | InlineStr / InlineVec (render, rhi, ecs-services, pool-utils-log), EngineStr / EngineVec (ecs-services), DiagArena (app-demo, ui-input), StrInterner (physics-scene-math) | The existing `boyko_log::DspBuf<N>`; structured errors (KF-10); plain fixed arrays; borrowed `&str`; KF-26 kernel name table; boyko_log structured records. |
| Bitsets, atomics, hash indexes | VmBitSet, VmBitMatrix, VmHashTable (ecs-schedule), VmAtomicWords, BoundedArray / InlineBits, FixedHashIndex (ecs-storage) | Existing LiveBitmap and `VmColumn<u64>`; KF-42 atomic views; ComponentMask / BitSet256; TypeIntern; a linear scan of the per-id layout table. |
| Scheduler and pool substrate | ChunkPool, ScopeSharedStack, BootArray, DetachedCellSlab (pool-utils-log); ScopeSharedSlab + ScopeBlockVmChunks, VmMpscRing, EpochIdBuffer, DynArena (ecs-schedule) | scope-arena on block.rs (KF-33); kernel-internal inline arrays (KF-31); a completion ring on a fixed VmColumn; a TermList protocol change; Commands (event) + components. DynArena is PARTIALLY kept as KF-07. |
| Static descriptors | StaticIdArena (ecs-schedule), per-type inline static and StaticDescriptorArena (macros-aether) | kernel-internal static tables (KF-43). |
| Miscellaneous | SliceSink (physics-scene-math), ZoneSlotTable (app-demo), engine-owned per-thread context slot and inline-by-value (ecs-storage), VmSparseMap / VmSparseSlotMap (pool-utils-log) | `&mut [T]` out-params with bounds known before the build; the reducer's ScratchColumn lanes; explicit context (TriggerContext, a world cell); a world-level `VmColumn<DeviceColumn>`; SparseMap moved into boyko_ecs on VmColumns; the RHI registry as a kernel generational table (KF-36). |
| KEPT (not refuted) | ComponentPool::add_uninit (ecs-services), boyko_mem (pool-utils-log), InjectorQueue (pool-utils-log), the remainder of DynArena (ecs-schedule) | Kept as, respectively, an API addition (KF-39), a layering feature (KF-32), kernel-internal pool storage (KF-34), and the erased record column (KF-07). None of them is a new column beside ComponentPool except KF-07, which generalises the CommandQueue record format. |

**Net result.** No new column primitive stands beside ComponentPool. What survives:

- KF-07, the erased record column, which generalises the existing CommandQueue record format.
- KF-32, a layering move of the memory library below the pool; KF-45's thread-context column lives there.
- KF-34, in-house pool lanes that replace crossbeam.
- API additions to existing types.

## Plan conflicts

These are the cross-cutting conflicts, merged from the eleven groups. Every group's own list follows verbatim.

1. **ALLOCATOR-DESIGN-SPACE (untracked rev-1 draft in the main checkout) against this ledger.** The design proposes new primitives beside ComponentPool: HeapVec, HeapDyn, DropColumn, FrameArena/FrameVec, TableSet, ByteColumn, InlineStr/HeapString. This ledger places those same rows in ECS forms on the existing ComponentPool / ScratchColumn / relation / event storage. The design's own critique already blocks HeapVec on Tree Borrows (C1). Every group recorded its instances: ecs-storage PC3/PC4/PC7, ecs-schedule, ecs-services, pool-utils-log, render, codec-tools, ui-input PC1/PC2, and app-demo. **Needed:** rev 2 of the design, re-based on the ECS forms. The central fork both sides agree on stays uncontested: replacement per TYPE by LIFETIME CLASS, allocator_api rejected, and `#[global_allocator]` kept only as a deny gate.
2. **Exception grants that predate the 2026-09-10 order and have not been retracted.** These are:
   - ARCH-AUDIT-ECS-DATA-REMEDIATION.md:9 (the LEGIT list) and its Stage 6 (accepted cold registries);
   - MEMORY-SYSTEM-AUDIT's OK-by-design grades;
   - DENSE-COMPONENTS-PLAN.md:17 (the DenseStore free list);
   - CLAUDE.md principle 0's "truly transient function-local scratch";
   - the in-code exception texts in prefab.rs:28-29, bindless.rs and retired_gpu_buffers.rs.

   The order supersedes all of them, but their texts still say otherwise and will need rewriting when the rows move.
3. **Children / relationship backing has four positions:** the backing is swappable (RELATIONS-API-PLAN), keep it until measured (MEMORY-SYSTEM-AUDIT), HeapVec (the design), and intrusive links (this ledger, KF-11).
4. **Physics** (physics-scene-math has the full text):
   - SoftBody columns: the census gate text, D-8, the inventory's RaggedColumn and the design each give a different answer. This ledger follows D-8.
   - Soft-body per-substep scratch: one shared column or rows in the bank.
   - IslandSleep form.
   - The OPTIMIZATION-PLAN per-island latch against the per-row code.
   - RefcountDeltas: event here, a plain Resource in ASSET-STREAMING-PLAN.
5. **Scope arena.** Three groups each place ScopeShared differently:
   - ecs-schedule: the block's first record;
   - pool-utils-log: a frame-local;
   - the design: the arena mark.

   The design also keys arenas by worker id, which its own critique C2 refutes.
6. **Demo body mirrors.** AUDIT-2026-07-PLAN says port them to dense. MEMORY-SYSTEM-AUDIT says keep them. ARCH-AUDIT and DENSE-COMPONENTS W4 say keep the gather. This ledger picks dense-component and records the gather as the fallback.
7. **Inconsistencies inside the rev-1 ledger, found by its synthesis** (all closed in rev 2: section Decisions and the change log) (counted from the rows):
   - D rows get three different forms;
   - T rows get four;
   - the scratch-id minting route differs by group;
   - the system-scratch backing (ScratchColumn or FrameArena) is open;
   - `SystemMeta::gpu_intent` is a component in the ECS pass but Resource+VmColumn in the recount;
   - app-demo's 38 std-internal allocation sites are carried as *supplementary* rows, while other groups carry the same mechanism (std::fs Windows path conversion, std::env) as ordinary T rows.

8. **Rev 2: the ledger against the two unification designs.** Where they disagreed, rev 2 decides by performance and records it here.
   - Sleep: rev 1 had an EnableTag `Sleeping`; physics Q3 / D4 put it in `BodyGate`. Rev 2 follows Q3 (W3); KF-16 and KF-18 are withdrawn.
   - Relation reverse index: rev 1 proposed intrusive links (KF-11); the engine design has K7 spans (EK15c). Rev 2 follows EK15c, on traversal cost.
   - Same-frame events: ui-lane UL-D4 builds KF-24; engine ED9 rejects a same-frame mode and raises triggers. Rev 2 follows ED9 (W5).
   - Fonts and sprite sheets: ui-lane UL-D3 keeps resource-columns; engine Q1 makes them asset entities. Rev 2 follows Q1 for the per-asset record and keeps the glyph CSR bytes as resource-columns (W4).
   - Asset stores: rev 1 proposed `Assets<T>::adopt_retiring` / `iter_mut` (KF-37/38); engine Q1 retires `Assets<T>` for K3 groups plus K6'. Rev 2 follows Q1; the fill-reject leak is fixed now with the existing queues (defect B).
   - Thread context: gap 1 cited a physics decision (RunCtx trampoline) that was superseded in place the same morning; KF-45 is corrected to the slot route (W7).
9. **Rev 2: the lanes against their own plans.** ui-lane: UI-PLAN-ANIMATION-DECISIONS.md:421 keeps the tween completion list as a std Vec in a Resource, and UI-ADVANCED-ARCHITECTURE.md:669 keeps the sheet table as a std Vec in a Resource; both predate the 2026-09-10 order. reflect-lane: REFLECTION-PLAN-CORE.md:1579 (the `validate` Vec signature), the in-code exception text at type_info.rs:420, and the planned heap sites of REFLECTION-PLAN-ECS.md:1665 and REFLECTION-PLAN-BOUNDARY.md:278/602. The group files carry the verbatim entries.
10. **Rev 3: the ledger against its own rev-2 recheck.** The recheck found the delta arithmetically sound and incomplete in seven places; rev 3 closes each (change log, rev 2 -> rev 3):
   - the ui-lane census stopped at crates/boyko_ui/src while the lane also rewrote crates/boyko_render/src/ui and added shaderdsl emit code (item 1);
   - engine Q1 was adopted in the index but not applied to 34 asset rows (item 2);
   - KF-24's rejection had one row without a stated delivery edge (item 3, now verified);
   - `diagnostics` held numeric captures and per-frame text (item 4);
   - 38 std-internal sites were uncounted, one of them per frame (item 5);
   - the path-conversion decision was applied two ways (item 6);
   - the rung rule put 11 class-K boyko_ecs rows after physics (item 7).

Per-group conflict lists, verbatim: [ecs-storage](ledger/ecs-storage.md#plan-conflicts), [ecs-schedule](ledger/ecs-schedule.md#plan-conflicts), [ecs-services](ledger/ecs-services.md#plan-conflicts), [pool-utils-log](ledger/pool-utils-log.md#plan-conflicts), [physics-scene-math](ledger/physics-scene-math.md#plan-conflicts), [render](ledger/render.md#plan-conflicts), [rhi](ledger/rhi.md#plan-conflicts), [ui-input](ledger/ui-input.md#plan-conflicts), [app-demo](ledger/app-demo.md#plan-conflicts), [codec-tools](ledger/codec-tools.md#plan-conflicts), [macros-aether](ledger/macros-aether.md#plan-conflicts), [ui-lane](ledger/ui-lane.md#plan-conflicts), [reflect-lane](ledger/reflect-lane.md#plan-conflicts).

## Order of work

The owner ordered the kernel finished first. Rows are assigned to rungs by one mechanical rule (rev 2, gap 5; step 4 widened in rev 3, item 7), applied in this order:

1. `out-of-scope:*` rows are not in a rung.
2. `boyko_physics` rows go to rung 3.
3. `boyko_threadpool` rows and `scope-arena` rows go to rung 5.
4. **`boyko_ecs` rows go to rung 2 when they are `kernel-internal` (any class), class K, or a row of an active kernel feature (except class D and T)** (kernel-internal since rev 2, gap 5; the rest since rev 3, item 7: no kernel row waits for physics).
5. Rows of class B, D or X, and rows in `diagnostics`, go to rung 6.
6. Other `kernel-internal` rows go to rung 2.
7. Everything else goes to rung 4.

A class-B row that constructs a field migrating in an earlier rung moves with that field in practice; the counts put it in rung 6 by its class.

|  | R2 | R3 | R4 | R5 | R6 | OOS | total |
|---|---|---|---|---|---|---|---|
| ecs-storage | 101 | . | 5 | . | 3 | . | 109 |
| ecs-schedule | 142 | . | . | 6 | 18 | . | 166 |
| ecs-services | 52 | . | 5 | . | 19 | 1 | 77 |
| pool-utils-log | 6 | . | 15 | 32 | 12 | 66 | 131 |
| physics-scene-math | . | 73 | 7 | . | 15 | . | 95 |
| render | 1 | . | 11 | . | 146 | 2 | 160 |
| rhi | . | . | 39 | . | 18 | 7 | 64 |
| ui-input | . | . | 8 | . | 75 | . | 83 |
| app-demo | . | . | 25 | . | 202 | 29 | 256 |
| codec-tools | . | . | 2 | . | 54 | 471 | 527 |
| macros-aether | 5 | . | . | . | . | 457 | 462 |
| ui-lane | 5 | . | 44 | . | 95 | 52 | 196 |
| reflect-lane | 2 | . | . | . | 5 | 17 | 24 |
| **total** | **314** | **73** | **161** | **38** | **662** | **1102** | **2350** |

**Rung 1: the kernel features physics needs (0 rows of its own).** In the physics design's order: K1 (U1) -> K2 + K3 + K6 (U2) -> K4 (U3) -> K5a/b (P1/P2) -> K7 (S0). They realise ledger KF-01, KF-03, KF-11, KF-19, KF-20, KF-21, KF-22, KF-49. The ledger adds physics needs the design does not order: KF-04 Durable resource column (lifetime + serialization), KF-23 Event lane policies (lossless / drop-oldest / coalesce / non-system producers), KF-45 Engine thread-context column (replaces every thread_local!). Together the rung-1 features unblock 370 active rows, app-demo 18, ecs-schedule 44, ecs-services 13, ecs-storage 43, physics-scene-math 91, pool-utils-log 6, render 9, rhi 32, ui-input 24, ui-lane 90. KF-16 and KF-18 left rung 1 in rev 2 (withdrawn by physics Q3).

**Rung 2: kernel-internal rows (314).** The ECS's own bookkeeping, the scheduler tables, the event and command channel storage, the static descriptors and the thread-context statics move onto the memory library. Since gap 5, every boyko_ecs kernel-internal row is here whatever its class; since rev 3, so is every boyko_ecs class-K row and every boyko_ecs row whose destination is a kernel feature (the scheduler's relation and dense rows, the observer stores, the query scratch, prefab and clone scratch, the asset kernel under engine Q1). It depends on the packing plan (docs/ecs/POOL-SUBGRANULAR-PACKING-PLAN.md): per-instance columns cost a 64 KiB commit granule each today and 4 KiB after it.

**Rung 3: physics (73 rows).** The rows move into the physics design's forms: `BodyGate` (W3), the K7 SoftBody segments (gap 3), the slot-keyed `PairCache` (gap 2), and the solver scratch that is already on ScratchColumn. At the same time the physics stages become parallel systems over dense kernel columns (K3/K4/K5). The serial fraction behind the Jolt residual (2.5x at W8, about 45 % serial) sits in exactly these stages.

**Rung 4: rows in entity/system forms that are not kernel work (161).** Components, dense components, relations, events, enable-states, resource-columns and system-scratch outside physics, including the whole UI, plus 10 boyko_ecs rows that are neither kernel-internal, class K, nor a kernel-feature row (the typed event lanes, archetype-edge and bundle scratch, the NonSend resource table, a class-T path conversion). Rev 2's text said 'the other crates' rows' while 70 boyko_ecs rows sat here; rev 3 moves the kernel ones to rung 2 and names the rest.

**Rung 5: scope arena and pool (38 rows).** block.rs onto the memory library (KF-32/33), the in-house lanes (KF-34), the pool's single reservation (KF-31) and the pool's thread-local statics (KF-45). Justified by unification; the heap A/B measured it as no speed lever.

**Rung 6: setup, diagnostics and X last (662 rows).** Boot/load-path rows (B), the `diagnostics` form, and FFI handoff buffers (X).

**Out of scope (1102 rows):** out-of-scope:compile-time 1021, out-of-scope:test-only 40, out-of-scope:os-owned 31, out-of-scope:third-party 10.

Check: R2 314 + R3 73 + R4 161 + R5 38 + R6 662 + out-of-scope 1102 = 2350 = total 2350.

Rung counts through rev 3 (rev 1 and the gaps: the eleven groups; rev 2 and rev 3: all active rows):

| rung | rev 1 (2193 rows) | after the gaps (2223) | rev 2 (2244 active) | rev 3 (2350 active) |
|---|---|---|---|---|
| R2 | 103 | 221 | 223 | 314 |
| R3 | 71 | 73 | 73 | 73 |
| R4 | 202 | 198 | 202 | 161 |
| R5 | 35 | 38 | 38 | 38 |
| R6 | 657 | 662 | 667 | 662 |
| OOS | 1125 | 1031 | 1041 | 1102 |

## Decisions (formerly undecided)

Rev 1 closed with 18 undecided forms, several marked "owner decision". Under the owner's 2026-09-11 delegation every one is decided here, by performance; where two options cost the same on every hot path, by the unification goal. Each names what would overturn it. "Source" says who decided: a gap, a lane, a decisions file, or this writer.

| question | rows | source | decision |
|---|---|---|---|
| System entities (KF-14) | 26 | writer | Systems, condition systems and sets are hidden entities (SystemBox a dense component, GpuAccessIntent and names components, InSet / Before / After / RunIf relations). The compiled executor tables stay kernel-internal behind SystemIndex -> entity. |
| Observers as entities (KF-15) | 5 | writer | Observers are entities with an Observer component; entity-targeted observers link through an `Observes` relation whose despawn cascade replaces the generation recycle guard. The global dispatch tables become derived contiguous indexes. |
| Prefab templates as entities (KF-13) | 8 | writer; = engine EK18 | Yes: templates are default-excluded entities; instantiate is clone_subtree plus marker removal. |
| Windows as entities (KF-46) | 13 | gap 8; engine Q3 | One entity per window; per-window data components; the OS-held input ring in a dense slot. |
| RHI layering (KF-36) | 44 | writer | Route (a): an rhi -> boyko_ecs edge. The RHI owns kernel columns and runs as NonSend systems; ARCHITECTURE.md is amended. |
| Soft-body per-substep scratch | 13 | gap 3; physics design | One shared system-scratch column sized to the largest body. |
| Demo balls and boids | 18 | writer | Keep the demo's 2D solver on dense kernel columns (dense-component + system-scratch, KF-19/20) rather than porting the balls onto boyko_physics. |
| ScopeShared placement | 10 | writer | The install frame: ScopeShared is a local of the frame that opens the scope, as the physics design places GangShared in lane 0's frame. The rows keep the form scope-arena; the kernel scope machinery owns the placement. |
| Propagation detach queue and KF-24 (rev 3) | 1 | rev 3 item 3; engine ED9 | An event delivered at the ordering edge: the ChildOf on_remove observer appends to the propagation system's lane inside the producer's apply window, and the executor applies and drains that window before it decrements the producer's successors (verified on joltab, KF-24 section). KF-24 stays rejected. |
| Rung rule step 4: kernel first (rev 3) | 297 | rev 3 item 7; the owner's order | Every boyko_ecs row that is kernel-internal, class K, or a row of an active kernel feature (except D and T) is in rung 2, before physics. |
| Supplementary rows (rev 3) | 38 | rev 3 item 5 | Counted rows. Boot env reads: out-of-scope:os-owned. Diagnostic env reads: diagnostics. Dump and sink path conversions: resource-column (the path rule). The per-frame BOYKO_VB_FORCE_CLASSIFIED read: a boot-read resource field. |
| hover_entered | 1 | writer (W5); engine ED9 | An event in its trigger form (Commands::trigger); KF-24 is not built. |
| SystemMeta::gpu_intent | 2 | follows KF-14 | A component on the system entity. |
| Children / relationship reverse index (KF-11) | 4 | writer; = engine EK15c | Per-target spans of a K7 segmented column, not intrusive links. |
| Kernel free lists (KF-41) | 4 | writer | Intrusive dead-slot lists where the dead slot has a free word; a VmColumn stack where the slot must hold DEAD bytes (physics K3 dying list). |
| Fonts and sprite sheets | 18 | writer (W4); engine Q1 | Asset entities of table kinds: the per-asset record is a component; the glyph tables are write-once CSR bytes on resource-owned columns. UL-D3 is overturned by scope. |
| SparseMap users | 9 | both groups agree | SparseMap moves into boyko_ecs on VmColumns; users leave through their own forms; block_groups is flattened to Copy ranges (no KF-02). |
| Scratch id minting route (KF-01) | 324 | ui-lane UL-D6; = physics K1 / engine EK1 | A registry-free scratch band that does not consume component ids, with a contiguous stagger run per cohort. |
| System-scratch backing | 370 | ui-lane UL-D5; engine EK13 deleted | ScratchColumn, per system, high-water retained, cleared by set_len. FrameArena is not built. |
| D rows: diagnostics form, narrowed (rev 3) | 376 | gap 6; W1/W2 for the lanes; rev 3 item 4 | Text and error payloads: the form `diagnostics`. Numeric captures (profiler samples, readback words, histograms, bit matrices): system-scratch when rebuilt per call, resource-column when retained across frames. Per-frame text: system-scratch, by its growth. |
| T rows: third-party / std-internal form | 44 | gap 1 (TLS), writer (W6) | thread_local! statics: kernel-internal via KF-45. crossbeam internals: kernel-internal via KF-34. std internals: out-of-scope:os-owned while boot-only. One path rule since rev 3 (item 6): a std path conversion reachable after steady state (the boyko_log sink rotation and on-demand opens, AssetServer::load, save / load, the ui hot-reload poll, the end-of-run dumps) is resource-column, in-house UTF-16 Win32 FFI on a path pre-encoded into the owning record. Third-party-imposed types: out-of-scope:third-party. |
| Asset rows under engine Q1 (KF-37 / KF-38; rev 3) | 34 | engine Q1; defect B; rev 3 item 2 | Assets are entities. GPU values are K3 Deferred group columns (dense-component) released through K6' (KF-49); the handle lists become in-place group walks; the refcount is a count-only relation (KF-48); Pinned is a marker component; live / free / dirty are the group's kernel bookkeeping; the staging queue is a Staged&lt;A::Cpu> component; decode payloads are load-system scratch. KF-37 superseded, KF-38 withdrawn. The fill-reject leak is fixed now with the existing Orphaned*Gpu queues. |
| Physics sleep form | 4 | physics Q3 / D4 (W3) | BodyGate in the PhysicsBody dense group, plus transition events. |
| Per-pair physics data | 2 | gap 2; physics D5 + D15 | resource-column PairCache keyed by stable body slots, double-buffered, fresh_step skip. |
| SoftBody storage | 36 | gap 3; physics Q2 | K7 segmented dense column. |
| thread_local! statics | 12 | gap 1; corrected by W7 | KF-45 thread-context column, reached by slot. |
| Exclusive-system scratch | 21 | gap 7; = engine EK2 | KF-44 Local&lt;T> for exclusive systems. |
| Matched archetype list | 15 | gap 9e | kernel-internal, held once per query in the QueryState cache. |

### System entities (KF-14) (26 rows)

- **Decision:** Systems, condition systems and sets are hidden entities (SystemBox a dense component, GpuAccessIntent and names components, InSet / Before / After / RunIf relations). The compiled executor tables stay kernel-internal behind SystemIndex -> entity.
- **Why (performance):** Equal on the hot path: the per-frame executor reads the same compiled tables either way (the feature's own design). The entity form costs only at schedule build. Tie-break: one unified system.
- **Overturned by:** The schedule-dispatch bench (the engine design's render-schedule budget of 2 us per frame at ~20 systems) regressing beyond band, or schedule build time beyond band.
- **Source:** writer
- **Current forms:** relation 18, component 6, dense-component 2
- **Rows:** crates/boyko_ecs/src/ecs/core/schedule/schedule.rs:122,161,172; crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs:106,116,122,127,131,139,153,155,156,157,158,159,682,683,693; crates/boyko_ecs/src/ecs/core/schedule/system_descriptor.rs:50,55,62,80,81,82; crates/boyko_ecs/src/ecs/core/system/system_meta.rs:140,331

### Observers as entities (KF-15) (5 rows)

- **Decision:** Observers are entities with an Observer component; entity-targeted observers link through an `Observes` relation whose despawn cascade replaces the generation recycle guard. The global dispatch tables become derived contiguous indexes.
- **Why (performance):** The fire path reads the derived contiguous index, so it keeps today's scan; the entity form is only the identity and lifetime.
- **Overturned by:** A trigger microbench (N observers per event) regressing beyond band.
- **Source:** writer
- **Current forms:** relation 5
- **Rows:** crates/boyko_ecs/src/ecs/core/component/observers/entity_store.rs:102,111,120,123,125

### Prefab templates as entities (KF-13) (8 rows)

- **Decision:** Yes: templates are default-excluded entities; instantiate is clone_subtree plus marker removal.
- **Why (performance):** The default-excluded marker is one mask test per archetype at query-state build, cached; zero per iteration. It closes the dense-membership gap.
- **Overturned by:** Query-state build time on a many-archetype world beyond band.
- **Source:** writer; = engine EK18
- **Current forms:** component 4, system-scratch 4
- **Rows:** crates/boyko_ecs/src/ecs/core/clone/prefab.rs:136,281,284,364,458,460,608,679

### Windows as entities (KF-46) (13 rows)

- **Decision:** One entity per window; per-window data components; the OS-held input ring in a dense slot.
- **Why (performance):** Equal per frame (touched once per window per frame); tie-break unification.
- **Overturned by:** none expected
- **Source:** gap 8; engine Q3
- **Current forms:** system-scratch 5, component 4, dense-component 2, event 2
- **Rows:** crates/boyko_rhi_vulkan/src/present/frame_driver.rs:48; crates/boyko_rhi_vulkan/src/present/surface.rs:180,238; crates/boyko_rhi_vulkan/src/present/swapchain.rs:64,66; crates/boyko_rhi_vulkan/src/window.rs:122,230,274,352,365,761; crates/boyko_input/src/raw/queue.rs:35,59

### RHI layering (KF-36) (44 rows)

- **Decision:** Route (a): an rhi -> boyko_ecs edge. The RHI owns kernel columns and runs as NonSend systems; ARCHITECTURE.md is amended.
- **Why (performance):** Equal: the same columns either way. Route (b) would give the RHI VmColumns but not ECS forms, a second storage API.
- **Overturned by:** A dependency cycle appearing (boyko_ecs needing boyko_rhi); not a performance gate.
- **Source:** writer
- **Current forms:** system-scratch 27, resource-column 11, component 4, dense-component 2
- **Rows:** 44, by group: pool-utils-log 3, rhi 41. The filter above identifies them.

### Soft-body per-substep scratch (13 rows)

- **Decision:** One shared system-scratch column sized to the largest body.
- **Why (performance):** Bodies are stepped serially, so one column is reused hot; per-body rows would multiply resident memory for no reuse.
- **Overturned by:** S0 stepping all bodies in one wave (then per-body segments co-slotted in K7).
- **Source:** gap 3; physics design
- **Current forms:** system-scratch 13
- **Rows:** crates/boyko_physics/src/soft/component.rs:77,79,81,129,131,133,142,144,146,154,163,171,177

### Demo balls and boids (18 rows)

- **Decision:** Keep the demo's 2D solver on dense kernel columns (dense-component + system-scratch, KF-19/20) rather than porting the balls onto boyko_physics.
- **Why (performance):** boyko_physics is a 3D solver; running it on 2D balls does more work per contact (reasoned, not measured). Both options are on ECS storage, so unification does not break the tie.
- **Overturned by:** boyko_physics measured within band of the demo solver at the demo's ball count: then port and delete the demo solver.
- **Source:** writer
- **Current forms:** dense-component 10, system-scratch 8
- **Rows:** crates/boyko_demo/src/sim/grid.rs:38,41,45,64,65,66; crates/boyko_demo/src/sim/resources.rs:134,142,191,193,195,199,203,211,212,213,214,217

### ScopeShared placement (10 rows)

- **Decision:** The install frame: ScopeShared is a local of the frame that opens the scope, as the physics design places GangShared in lane 0's frame. The rows keep the form scope-arena; the kernel scope machinery owns the placement.
- **Why (performance):** Zero allocation and no arena bump; the scope's join precedes the frame's return by construction.
- **Overturned by:** The KE16 completion-protector Miri gate, re-established RED-first (W1), finding a use after the frame returns: then the first record of the scope block.
- **Source:** writer
- **Current forms:** scope-arena 10
- **Rows:** crates/boyko_ecs/src/ecs/core/iters/query/par_chunk.rs:139,263; crates/boyko_ecs/src/ecs/core/iters/query/par_iter.rs:341,406; crates/boyko_ecs/src/ecs/core/schedule/schedule.rs:455,1341; crates/boyko_threadpool/src/block.rs:482; crates/boyko_threadpool/src/scope.rs:1096; crates/boyko_threadpool/src/thread_pool.rs:278,328

### Propagation detach queue and KF-24 (rev 3) (1 rows)

- **Decision:** An event delivered at the ordering edge: the ChildOf on_remove observer appends to the propagation system's lane inside the producer's apply window, and the executor applies and drains that window before it decrements the producer's successors (verified on joltab, KF-24 section). KF-24 stays rejected.
- **Why (performance):** One push per detach and no publish step; a same-frame event mode would add a publish step at every ordering edge for every event type.
- **Overturned by:** A detach consumer that must run unordered with its producer and still see the detach in the same frame.
- **Source:** rev 3 item 3; engine ED9
- **Current forms:** event 1
- **Rows:** crates/boyko_scene/src/propagation.rs:133

### Rung rule step 4: kernel first (rev 3) (297 rows)

- **Decision:** Every boyko_ecs row that is kernel-internal, class K, or a row of an active kernel feature (except D and T) is in rung 2, before physics.
- **Why (performance):** Not a speed decision: an ordering decision taken on the owner's order. It moves no cost onto a hot path; it only removes the case where a physics rung waits on kernel storage that was scheduled after it.
- **Overturned by:** none; a kernel row that turns out to depend on a physics feature moves with that feature.
- **Source:** rev 3 item 7; the owner's order
- **Current forms:** kernel-internal 210, system-scratch 40, relation 26, component 12, resource-column 4, event 3, dense-component 2
- **Rows:** 297, by group: ecs-schedule 142, ecs-services 52, ecs-storage 101, reflect-lane 2. The filter above identifies them.

### Supplementary rows (rev 3) (38 rows)

- **Decision:** Counted rows. Boot env reads: out-of-scope:os-owned. Diagnostic env reads: diagnostics. Dump and sink path conversions: resource-column (the path rule). The per-frame BOYKO_VB_FORCE_CLASSIFIED read: a boot-read resource field.
- **Why (performance):** The per-frame read costs a heap allocation and an environment lookup every frame; the others are boot-only or removed by the path rule.
- **Overturned by:** none expected
- **Source:** rev 3 item 5
- **Current forms:** out-of-scope:os-owned 19, resource-column 11, diagnostics 8
- **Rows:** crates/boyko_app/src/gpu_scene/mod.rs:4339,4544,6415; crates/boyko_app/src/host_dump.rs:67,237; crates/boyko_app/src/hzb_dump.rs:86,277; crates/boyko_app/src/particle_readback.rs:601; crates/boyko_app/src/plugins.rs:224,227,228,246,276,316,792,843,844; crates/boyko_app/src/profiling/artifact.rs:760; crates/boyko_app/src/profiling/stream.rs:218,245,246,248; crates/boyko_app/src/runner.rs:161,385,411,422,537,748,1060,1078,1110,2946; crates/boyko_app/src/vb_cull_probe.rs:97,204; crates/boyko_app/src/vb_probe_dump.rs:104,220; crates/boyko_app/src/vg_census_dump.rs:101,350

### hover_entered (1 rows)

- **Decision:** An event in its trigger form (Commands::trigger); KF-24 is not built.
- **Why (performance):** A handful of transitions per frame; no publish-at-edge machinery on every event type.
- **Overturned by:** A consumer that must read hover transitions from an ordered system, at a rate where the observer path measures costlier.
- **Source:** writer (W5); engine ED9
- **Current forms:** event 1
- **Rows:** [ui-lane] crates/boyko_ui/src/interaction/focus.rs:128

### SystemMeta::gpu_intent (2 rows)

- **Decision:** A component on the system entity.
- **Why (performance):** As KF-14.
- **Overturned by:** As KF-14.
- **Source:** follows KF-14
- **Current forms:** component 2
- **Rows:** crates/boyko_ecs/src/ecs/core/system/system_meta.rs:140,331

### Children / relationship reverse index (KF-11) (4 rows)

- **Decision:** Per-target spans of a K7 segmented column, not intrusive links.
- **Why (performance):** Traversal (propagation, layout, joint cleanup) reads one contiguous span; links cost one random access per child.
- **Overturned by:** A reparent-churn bench where links beat spans beyond band while the traversal benches stay within band.
- **Source:** writer; = engine EK15c
- **Current forms:** relation 4
- **Rows:** crates/boyko_ecs/src/ecs/core/component/observers/entity_store.rs:102; crates/boyko_ecs/src/ecs/core/hierarchy/mod.rs:120,159; crates/boyko_ecs/src/ecs/core/relationship/collection.rs:85

### Kernel free lists (KF-41) (4 rows)

- **Decision:** Intrusive dead-slot lists where the dead slot has a free word; a VmColumn stack where the slot must hold DEAD bytes (physics K3 dying list).
- **Why (performance):** No extra container, and the pop touches the line the reuse writes anyway. LIFO order preserved.
- **Overturned by:** A spawn/despawn churn bench beyond band.
- **Source:** writer
- **Current forms:** kernel-internal 4
- **Rows:** crates/boyko_ecs/src/ecs/core/archetype/archetype_bundle.rs:135; crates/boyko_ecs/src/ecs/core/archetype/archetype_registry.rs:11; crates/boyko_ecs/src/ecs/core/component/dense/dense_store.rs:126; crates/boyko_ecs/src/ecs/core/entity/entity_master.rs:73

### Fonts and sprite sheets (18 rows)

- **Decision:** Asset entities of table kinds: the per-asset record is a component; the glyph tables are write-once CSR bytes on resource-owned columns. UL-D3 is overturned by scope.
- **Why (performance):** Per glyph: identical. Per text node: one entity lookup instead of one table index, once per frame.
- **Overturned by:** The engine AS2 handle-resolution microbench, or the UI gather measuring the per-node lookup above the table index.
- **Source:** writer (W4); engine Q1
- **Current forms:** resource-column 15, component 2, system-scratch 1
- **Rows:** crates/boyko_fontbake/src/atlas.rs:116,126,128,130,598,611,619,629; [ui-lane] crates/boyko_ui/src/sprite.rs:257; [ui-lane] crates/boyko_ui/src/text/font.rs:33,36,38,41,52,55,72,75,139

### SparseMap users (9 rows)

- **Decision:** SparseMap moves into boyko_ecs on VmColumns; users leave through their own forms; block_groups is flattened to Copy ranges (no KF-02).
- **Why (performance):** Same probe cost.
- **Overturned by:** none expected
- **Source:** both groups agree
- **Current forms:** kernel-internal 6, system-scratch 2, relation 1
- **Rows:** crates/boyko_ecs/src/ecs/core/archetype/archetype_registry.rs:11,24; crates/boyko_ecs/src/ecs/core/clone/map.rs:22; crates/boyko_ecs/src/ecs/core/clone/prefab.rs:458; crates/boyko_ecs/src/ecs/core/component/component_pool_bundle.rs:14; crates/boyko_ecs/src/ecs/core/component/observers/entity_store.rs:120; crates/boyko_utils/src/sparse_map/sparse_map.rs:7,10,13

### Scratch id minting route (KF-01) (324 rows)

- **Decision:** A registry-free scratch band that does not consume component ids, with a contiguous stagger run per cohort.
- **Why (performance):** Off the production id counter; stagger explicit where it matters.
- **Overturned by:** Band exhaustion, or a measured conflict-miss rate a per-type id would have avoided.
- **Source:** ui-lane UL-D6; = physics K1 / engine EK1
- **Current forms:** system-scratch 212, kernel-internal 44, dense-component 37, resource-column 23, component 4, event 3, relation 1
- **Rows:** 324, by group: app-demo 7, ecs-schedule 44, ecs-services 7, ecs-storage 35, physics-scene-math 85, render 3, rhi 31, ui-input 22, ui-lane 90. The filter above identifies them.

### System-scratch backing (370 rows)

- **Decision:** ScratchColumn, per system, high-water retained, cleared by set_len. FrameArena is not built.
- **Why (performance):** Steady state does no work; one bump arena cannot grow several lanes at once.
- **Overturned by:** The resident commit of the scratch columns (64 KiB per column before the packing plan) over a memory budget.
- **Source:** ui-lane UL-D5; engine EK13 deleted
- **Current forms:** system-scratch 370
- **Rows:** 370, by group: app-demo 30, codec-tools 46, ecs-schedule 8, ecs-services 8, ecs-storage 38, physics-scene-math 46, render 50, rhi 41, ui-input 14, ui-lane 89. The filter above identifies them.

### D rows: diagnostics form, narrowed (rev 3) (376 rows)

- **Decision:** Text and error payloads: the form `diagnostics`. Numeric captures (profiler samples, readback words, histograms, bit matrices): system-scratch when rebuilt per call, resource-column when retained across frames. Per-frame text: system-scratch, by its growth.
- **Why (performance):** Cold text: I-cache and the emitting thread's time dominate, and deferred formatting moves the work to the drain. A capture is data an armed probe reads: a ScratchColumn lane reserved at its bound, or a resource column that lives until the report, does no growth reallocation.
- **Overturned by:** none expected
- **Source:** gap 6; W1/W2 for the lanes; rev 3 item 4
- **Current forms:** diagnostics 301, resource-column 51, system-scratch 22, out-of-scope:os-owned 2
- **Rows:** 376, by group: app-demo 182, codec-tools 1, ecs-schedule 14, ecs-services 16, ecs-storage 1, pool-utils-log 14, reflect-lane 5, render 80, rhi 2, ui-input 44, ui-lane 17. The filter above identifies them.

### T rows: third-party / std-internal form (44 rows)

- **Decision:** thread_local! statics: kernel-internal via KF-45. crossbeam internals: kernel-internal via KF-34. std internals: out-of-scope:os-owned while boot-only. One path rule since rev 3 (item 6): a std path conversion reachable after steady state (the boyko_log sink rotation and on-demand opens, AssetServer::load, save / load, the ui hot-reload poll, the end-of-run dumps) is resource-column, in-house UTF-16 Win32 FFI on a path pre-encoded into the owning record. Third-party-imposed types: out-of-scope:third-party.
- **Why (performance):** The TLS and crossbeam replacements remove measured per-read and per-thread costs. The path rule removes one allocation and one UTF-8 to UTF-16 transcode per call after steady state, and it keeps a deny-after-steady gate from aborting inside std.
- **Overturned by:** An msvc TLS A/B for KF-45; for the path rule, a counting #[global_allocator] showing std path calls no longer allocate (then those rows are deleted, not moved).
- **Source:** gap 1 (TLS), writer (W6)
- **Current forms:** resource-column 16, kernel-internal 14, out-of-scope:third-party 10, out-of-scope:os-owned 4
- **Rows:** 44, by group: app-demo 10, codec-tools 2, ecs-services 5, ecs-storage 2, pool-utils-log 23, ui-lane 2. The filter above identifies them.

### Asset rows under engine Q1 (KF-37 / KF-38; rev 3) (34 rows)

- **Decision:** Assets are entities. GPU values are K3 Deferred group columns (dense-component) released through K6' (KF-49); the handle lists become in-place group walks; the refcount is a count-only relation (KF-48); Pinned is a marker component; live / free / dirty are the group's kernel bookkeeping; the staging queue is a Staged&lt;A::Cpu> component; decode payloads are load-system scratch. KF-37 superseded, KF-38 withdrawn. The fill-reject leak is fixed now with the existing Orphaned*Gpu queues.
- **Why (performance):** The draw path does not resolve a handle per draw (engine Q1); structural changes happen at load and unload, not per frame; one store per kind replaces the duplicated tables and their sync step.
- **Overturned by:** The engine AS2 handle-resolution microbench.
- **Source:** engine Q1; defect B; rev 3 item 2
- **Current forms:** system-scratch 21, dense-component 7, kernel-internal 3, component 2, relation 1
- **Rows:** crates/boyko_ecs/src/ecs/core/asset/assets.rs:204,205,206,207; crates/boyko_ecs/src/ecs/core/asset/staging.rs:58; crates/boyko_scene/src/asset_refs.rs:99,149; crates/boyko_render/src/asset_refcount.rs:556; crates/boyko_render/src/gpu_upload.rs:216; crates/boyko_render/src/loaders/glb.rs:786,831,857,915,916; crates/boyko_render/src/loaders/obj.rs:183,187,188; crates/boyko_render/src/loaders/png_texture.rs:42,50; crates/boyko_render/src/mesh_assets.rs:582,712; crates/boyko_render/src/mesh_data.rs:28,30; crates/boyko_render/src/texture.rs:709,928; crates/boyko_render/src/texture_data.rs:28; crates/boyko_fontbake/src/atlas.rs:116; crates/boyko_image/src/png.rs:60,378,381,421,430,441,456

### Physics sleep form (4 rows)

- **Decision:** BodyGate in the PhysicsBody dense group, plus transition events.
- **Why (performance):** No extra load: the solver already streams the co-slotted group.
- **Overturned by:** None expected (the physics decisions file).
- **Source:** physics Q3 / D4 (W3)
- **Current forms:** dense-component 4
- **Rows:** crates/boyko_physics/src/resources.rs:3057,3062,3104,3105

### Per-pair physics data (2 rows)

- **Decision:** resource-column PairCache keyed by stable body slots, double-buffered, fresh_step skip.
- **Why (performance):** Equal probe cost; read-old/write-new unblocks a parallel narrowphase.
- **Overturned by:** The physics design's U7 and G-jolt gates.
- **Source:** gap 2; physics D5 + D15
- **Current forms:** resource-column 2
- **Rows:** crates/boyko_physics/src/narrowphase/axis_cache.rs:128; crates/boyko_physics/src/solver/warm_start.rs:214

### SoftBody storage (36 rows)

- **Decision:** K7 segmented dense column.
- **Why (performance):** Per-body contiguity; no per-particle bookkeeping.
- **Overturned by:** soft_colored_sp4 regressing beyond band.
- **Source:** gap 3; physics Q2
- **Current forms:** dense-component 36
- **Rows:** crates/boyko_physics/src/soft/component.rs:71,73,75,83,85,87,89,91,93,95,98,105,107,109,111,115,118,327,328,329,330,331,374,499,500,501,511,512,513,514,516,517,524,526,540,541

### thread_local! statics (12 rows)

- **Decision:** KF-45 thread-context column, reached by slot.
- **Why (performance):** One load plus an L1 probe against two contended RMWs plus FlsSetValue per read.
- **Overturned by:** An msvc A/B of worker/body_1us_tasks_64W.
- **Source:** gap 1; corrected by W7
- **Current forms:** kernel-internal 12
- **Rows:** crates/boyko_ecs/src/ecs/core/component/component_registry/required.rs:153; crates/boyko_ecs/src/ecs/core/component/hooks/scope.rs:31; crates/boyko_ecs/src/ecs/core/component/observers/propagate.rs:30; crates/boyko_ecs/src/ecs/core/hierarchy/commands.rs:127; crates/boyko_ecs/src/ecs/core/relationship/mod.rs:90,152; crates/boyko_diag/src/lane.rs:139; crates/boyko_log/src/drain_owner.rs:42; crates/boyko_log/src/sync_out.rs:75; crates/boyko_threadpool/src/tls.rs:169,196,205

### Exclusive-system scratch (21 rows)

- **Decision:** KF-44 Local&lt;T> for exclusive systems.
- **Why (performance):** No world-global resource lookup per run.
- **Overturned by:** none expected
- **Source:** gap 7; = engine EK2
- **Current forms:** system-scratch 20, event 1
- **Rows:** crates/boyko_scene/src/propagation.rs:120,124,133,404; [ui-lane] crates/boyko_ui/src/binding/bind_system.rs:46,48; [ui-lane] crates/boyko_ui/src/interaction/focus.rs:117,119,122,131,136; [ui-lane] crates/boyko_ui/src/resources.rs:216,218,226,234,239,247; [ui-lane] crates/boyko_ui/src/widgets.rs:73; [ui-lane] crates/boyko_ui/src/world/pick.rs:110,113,117

### Matched archetype list (15 rows)

- **Decision:** kernel-internal, held once per query in the QueryState cache.
- **Why (performance):** No per-frame archetype rescan on a stable world.
- **Overturned by:** A caller that measurably needs a fresh one-off scan per frame.
- **Source:** gap 9e
- **Current forms:** kernel-internal 15
- **Rows:** crates/boyko_ecs/src/ecs/core/archetype/archetype_registry.rs:126,149,162,180,199,226,317,335,396,419; [ui-lane] crates/boyko_render/src/ui/gather.rs:286; [ui-lane] crates/boyko_ui/src/binding/bind_system.rs:50; [ui-lane] crates/boyko_ui/src/interaction/focus.rs:133; [ui-lane] crates/boyko_ui/src/widgets.rs:75; [ui-lane] crates/boyko_ui/src/world/pick.rs:120

### The lanes' own decisions

The ui-lane decided UL-D1 to UL-D7 and the reflect lane six questions (their group files carry the full text). Rev 2 keeps all of them except where noted:

- **UL-D1** D rows: diagnostics form (ledger section (i), a vocabulary decision): kernel-internal (boyko_log structured record: code + tagged args). **Rev 2:** superseded by gap 6 (form `diagnostics`), W1.
- **UL-D2** T rows: std-internal form (ledger section (i), a vocabulary decision): resource-column: in-house GetFileAttributesExW on a UTF-16 path encoded once into UiHotReload (inline array, no heap). **Rev 3:** kept for the runtime poll; the boot call plugin.rs:132 is out-of-scope:os-owned under the one path rule (item 6).
- **UL-D3** Fonts: resource-column vs kernel Assets&lt;T> (ledger section (i), 'the text/asset owner'); extended to the lane's sprite sheets: resource-column on the FontTable / UiSheetTable Resources (KF-01 + KF-04). **Rev 2:** overturned for the per-asset record by engine Q1 (W4); the glyph tables stay resource-column.
- **UL-D4** hover_entered: event vs system-scratch (ledger section (i), decided by KF-24): event; KF-24 is built (system-scratch until it exists). **Rev 2:** the event stands; "KF-24 is built" is superseded by engine ED9 (W5).
- **UL-D5** System-scratch backing: ScratchColumn vs FrameArena (ledger section (i), 'Design rev 2'): ScratchColumn (per-system, high-water retained, cleared by set_len).
- **UL-D6** Scratch id minting route (KF-01; ledger section (i), 'One kernel decision'): kernel scratch band keyed by Layout, explicit stagger argument.
- **UL-D7** The lane's tween completion list (new row animation.rs:473): where does a finished channel's removal travel?: event on the kernel command channel: ui_visual_tick issues Commands::remove::&lt;Tween*>(entity); UiTweenScratch and ui_tween_reap are deleted.
- **reflect** D-row form for this group's validate rows (ledger section (i) "D rows: diagnostics form"): kernel-internal (diagnostics substrate), realised by eliminating the buffer: validate_with visitor + count-returning validate. **Rev 2:** form `diagnostics` (gap 6), W2; the visitor mechanism stands.
- **reflect** CORE D16 - one table (reflect slots inside BIND_ACCESSORS) or two (REFLECT separate); "OWNER decision, the plan proceeds on Horn 2": Horn 2: two tables.
- **reflect** REFLECTION-ANALYSIS B.12 option (b) / owner sheet B.13 #1 - engine crates carry a non-default `reflect` feature + optional edge ("the plan proceeds on (b) pending the owner", crates/boyko_scene/Cargo.toml): option (b).
- **reflect** ECS form of the reflection registry REFLECT itself (types as entities, flecs-style, vs a static table): kernel-internal static table (not a heap site, so not a row).
- **reflect** EG6 add_default #[cold] heap fallback (planned): KF-05 world scratch frame; delete the fallback if the measured max layout fits the stack scratch.
- **reflect** CORE C11 String arm (planned): Str accessor over engine text carriers (inline bytes / KF-26 id / KF-03 span), no std String write path.

## Defects

Two latent defects came out of the ledger work. Each was traced in code on `D:/wt/joltab` at HEAD `d11962a9` (crates/*/src identical to `ca582e72`); nothing was built or run.

### A. Row-keyed physics state - CONFIRMED, and worse than rev 1 said

A body's solver row is its position in the gather's walk over matching archetypes, and a despawn swap-removes that row: joltab:crates/boyko_ecs/src/ecs/core/archetype/archetype.rs:1268 "self.entity_ids.swap_remove(removed_unit_index.0);". The sleep latch is only resized, never re-keyed: joltab:crates/boyko_physics/src/resources.rs:3187 "self.asleep.resize(n_rows, false);". The docs claim the opposite: joltab:crates/boyko_physics/src/resources.rs:3013 "/// splitting. Body ROWS, by contrast, are STABLE across frames (the gather is FULL".

- **A1 - unbounded.** A body spawned into a latched row (despawn P_j with j < n, so Pn moves into row j, then spawn E at rest in mid-air; or the reverse order) inherits `asleep = true`, its one-member island stays frozen, gravity is undone on restore, and it never moves until an awake body touches it. Rev 1 and the physics research called this "frozen for one frame".
- **A2 - bounded, performance.** An unrelated despawn moves a sleeping pile member into an awake row: the whole pile island is solved for another `sleep_frames` steps (60 by default).
- **A3.** A fast mover freezes for one step (the research's F-3). With bodies over several archetypes, any structural change in a non-last archetype shifts every later row by one.
- **Warm start - CONFIRMED on the default path.** Keys are built from rows (joltab:crates/boyko_physics/src/solver/colored.rs:1664 "warm_start::pack(m.body_a, m.body_b, cp.feature_id)"), sphere contacts share feature id 0, and warm start is on by default (joltab:crates/boyko_physics/src/solver/colored.rs:1456 "warm_start_enabled: true,"). A survivor moved into a despawned body's row reads that body's stored impulse for one step: a wrong HIT, not the MISS the module doc claims. This is the gap's DC-PAIR-1, now confirmed.
- **BoxAxisCache - key reuse CONFIRMED, REFUTED as a correctness defect:** an inherited axis is kept only within 5 % of the best depth (joltab:crates/boyko_physics/src/narrowphase/box_box.rs:246 "Some(last) if last.index != best.index && best.depth >= last.depth / HYSTERESIS_RATIO => last,").
- **Exposure.** Sleeping is off by default (joltab:crates/boyko_physics/src/resources.rs:480 "sleeping: false,"); warm start is on.

**Decision (performance first, bugs before features).**

1. **Latch: partial fix now.** Clear the latch on any row whose `RigidBody` was added since the solver last ran, in a pass that runs only when sleeping is on, walking in gather order. It removes A1 in both orders. Overturned by `benches/sleeping.rs` regressing beyond noise (then fold the read into the gather). A2 and the one-step freeze remain until the physics U6 rung.
2. **Warm start: deferred to physics U5 + U7** (slot identity, `fresh_step`). The gather cannot identify a swap-moved body today, and an interim identity would be thrown away at U5. Overturned by the warm-start red-first test showing a survivor velocity jump above 10x resting noise, or visible pops: then an interim 16 B/row last-position column marks the row fresh.
3. **Land now:** three device-free red-first tests on the `sleeping_pipeline_o8.rs` harness (A1 in both orders: E's y < 3.0 after 30 steps, today exactly 4.0; A2: every pile row asleep after the step; warm start: the survivor's velocity matches a control run), and corrections of the false claims at `resources.rs:3013-3014, 3055-3056, 3173-3177`, `warm_start.rs:35-41`, `axis_cache.rs:28-34`.
4. **The physics design covers it structurally** (D1 stable slots at U5, D4 `BodyGate` whose DEAD value means awake at U6, D15 `fresh_step` at U7). Widen U6's red-first gate from F-3 to A1 (both orders), A2 and the cross-archetype shift. In the ledger, writer change W3 puts the latch in `BodyGate`.

### B. Rejected GPU uploads leak device memory - CONFIRMED in code, latent in practice

The only production `Assets::fill` caller discards the rejected value: joltab:crates/boyko_render/src/gpu_upload.rs:120 "let _ = assets.fill(staged.handle, gpu);". Dropping a `MeshGpu` frees nothing (joltab:crates/boyko_render/src/mesh.rs:131 "/// `MeshGpu` does NOT implement `Drop`: an RHI [`BoundBuffer`] must be destroyed through"), and the fill doc names the obligation (joltab:crates/boyko_ecs/src/ecs/core/asset/assets.rs:450 "/// device buffers/BLAS it holds leak."). For meshes the lost value holds a vertex and an index buffer, a BLAS under `hwrt`, and a geometry-table slot when that table is armed; for textures a `VulkanTexture` and a bindless slot.

- **How a fill is rejected.** Between `AssetServer::load` (the row is Loading) and the boot drain, the row leaves Loading by (a) `assets.remove`, (b) the last MeshRef dropping to zero on the Loading row, or (c) the same handle staged twice. The drain then allocates and `fill` fails with StaleHandle.
- **Reachability.** The drain runs once, at boot, after every startup system; no in-tree scene calls `load`. (a) and (c) are reachable from any user startup system; (b) at boot was not established. All three become the normal streaming race once the drain runs every frame.
- **The queue built for this has no producer:** joltab:crates/boyko_render/src/mesh_assets.rs:729 "/// (no `fill` caller exists in-tree yet, so this is always `true` today).".

**Decision: fix now; the path is cold, so the fix costs nothing on success.** Route the `Err` value into the existing `OrphanedMeshGpu` / `OrphanedTextureGpu` queues (inserted, drained behind the fence gate, force-drained at shutdown), and make `OrphanedMeshGpu::drain_ready` unregister a non-reserved `geometry_slot`. Under engine Q1 the end state is the K6' group release (KF-37 superseded). Red-first: a `gpu:` device test on the f6 churn harness (reserve, stage, remove in a startup system; assert the orphan queue is non-empty, and no undestroyed VkBuffer at teardown with validation on); a device-free tripwire (`clippy::let_underscore_must_use` on `gpu_upload.rs:120`) checks text only. Not traced further: `MeshGeometryTable::unregister` (joltab:crates/boyko_render/src/mesh_geometry_table.rs:758 "pub fn unregister(&mut self, slot: u32, retire_frame: u64) {") has no caller, so under a VisibilityBuffer boot a normally retired mesh may never release its slot.

## Change log

### Rev 1 -> rev 2, by gap (the eleven groups; `rev2/gap_changes.json`)

| gap | records | distinct rows | fields | what |
|---|---|---|---|---|
| GAP1-thread-local | 15 | 15 | (row added) 12, form_note 3 | Every thread_local! static in non-test code is a row (31 statics, 31 rows): 12 rows added (9 shipped, class T, kernel-internal; 3 non-shipped), 4 false non_row reasons corrected. One decision: KF-45. |
| GAP2-pair-cache | 3 | 2 | (row added) 2, physics_entity_model(+PairCache) 1 | Rows WarmStartTable::slots and BoxAxisCache::slots added: class E, resource-column PairCache keyed by the stable body slot (D5 + D15). Defect candidate DC-PAIR-1 (confirmed by defect A for warm start). |
| GAP3-softbody-K7 | 109 | 36 | ecs_form 36, plan_ref 36, form_note 36, physics_entity_model[SoftBody] 1 | 36 SoftBody rows from resource-column to dense-component in the K7 segmented dense column (physics Q2). Per-substep scratch stays shared system-scratch. |
| GAP4-driver-owned | 69 | 20 | ecs_form 20, form_note 20, owning_entity 9, rung 20 | 20 out-of-scope:driver-owned rows re-tagged: 8 resource-column, 12 system-scratch; 0 remain. |
| GAP5-kernel-rung2 | 104 | 104 | rung 104 | New rung rule: boyko_ecs kernel-internal goes to rung 2 whatever its class. 104 rows R6 -> R2. |
| GAP6-diagnostics | 697 | 318 | ecs_form 318, form_note 318, rung 61 | New form `diagnostics`: 318 rows (257 from kernel-internal, 61 from out-of-scope:diagnostics). |
| GAP7-exclusive-local | 20 | 20 | form_note 20 | KF-44 (exclusive-system Local, = engine EK2) owns the scratch of 20 rows (UI 17, scene 3). |
| GAP8-window-entities | 19 | 13 | ecs_form 6, form_note 13 | All 13 window rows decided; KF-46 added; 6 changed form. |
| GAP9-minor | 173 | 57 | (row added) 16, class 30, ecs_form 39, form_note 41, owning_entity 8, rung 39 | prof_decode 15 rows and the profiling allocator row added; 30 artifact/contrast rows and dense_store.rs:794 C test-only -> D diagnostics; KF-11 physics=yes; one form for the matched-archetype list; the StackFrame Copy precondition. |

Rows: 2193 -> 2223. The gap agent's independent verifier (`rev2/gapwork/verify_gaps.py`) reported 0 errors and 2147 citations read back verbatim.

### The two lanes

- **ui-lane** (D:/wt/ui @ 615cda8f): 124 rows = the 122 boyko_ui rows of ui-input, re-anchored to lane lines, plus 2 new (UiTweenScratch::done, UiSheetTable::sheets). All 122 ui-input boyko_ui rows are SUPERSEDED in rev 2 (27 changed per the lane report: lane 8, lane+decision 2, decision 17; 95 carried unchanged). Seven decisions UL-D1 to UL-D7. 11 joltab-only non_rows (serialize.rs helper params) are absent from the lane: joltab-only code, not lane removals.
- **reflect-lane** (D:/wt/reflect @ 0e0b4c68): 19 rows (C 12 compile-time in the reflect derive, D 5 in validate, F 2 SmallList4 spills in the by-id seam), 161 non_rows, 0 superseded. One new kernel feature (KF-47).

### Writer changes (W1-W8; `rev2/synth2/writer_changes.json`)

| change | records | rows / items | what |
|---|---|---|---|
| W1-ui-lane-merge | 53 | 36 | The lane was built from the rev-1 ui-input rows, so it lacked the gap changes: 17 D rows -> diagnostics (gap 6), 17 gap-6 note suffixes, 17 gap-7 notes three-way merged into the lane's re-anchored notes (0 conflicts), 2 gap-9 notes. |
| W1-ui-lane-D-reconcile | 17 | 17 | A reconciliation note on the 17 lane D rows: UL-D1 was taken against the rev-1 vocabulary. |
| W2-reflect-lane-D | 10 | 5 | 5 reflect D rows kernel-internal -> diagnostics; the visitor mechanism stands. |
| W3-physics-Q3-BodyGate | 15 | 7 | IslandSleep asleep / below_count and their constructors: enable-state / component -> dense-component (BodyGate, physics Q3 / D4). Three physics-entity-model notes (D1/Q5, Q3, Q1). |
| W4-engine-Q1-asset-entities | 14 | 10 | FontTable::fonts and UiSheetTable::sheets: resource-column -> component on the asset entity (engine Q1); a note on the 8 other font rows. |
| W5-engine-ED9-trigger | 1 | 1 | hover_entered: note that it is raised as a trigger (engine ED9); KF-24 rejected. |
| W6-T-vocabulary | 4 | 2 | codec-tools save.rs:717 / load.rs:312: out-of-scope:third-party -> out-of-scope:os-owned (std, not third-party). |
| W7-stale-citation-decisions-89 | 19 | 10 | The physics decisions file line 89 (RunCtx trampoline) was superseded in place after gap 1 cited it: 19 citations replaced by the current line, plus a correction of the KF-45 route. |
| W8-reanchor-ALLOCATOR-DESIGN-SPACE | 1480 | 1428 | ALLOCATOR-DESIGN-SPACE.md was rewritten at 05:51:50, after rev 1: 1614 citations re-anchored by +21 lines, each quote re-found in the current file. |

Form changes by the writer: W1-ui-lane-merge: 17 x kernel-internal -> diagnostics; W2-reflect-lane-D: 5 x kernel-internal -> diagnostics; W3-physics-Q3-BodyGate: 2 x component -> dense-component; W3-physics-Q3-BodyGate: 2 x enable-state -> dense-component; W4-engine-Q1-asset-entities: 2 x resource-column -> component; W6-T-vocabulary: 2 x out-of-scope:third-party -> out-of-scope:os-owned.

### Rev 2 -> rev 3, by recheck item (`rev3/changes.json`)

| item | records | rows / items | what |
|---|---|---|---|
| item1 | 241 | 163 | ui-lane census outside crates/boyko_ui (every file the lane added plus the added hunks of the files it changed; crates/boyko_render/src/ui whole): 65 new rows, 73 non_rows; 9 joltab render rows and 11 render non_rows superseded. |
| item1-carried | 7 | 7 | The 7 joltab render rows carried onto their lane lines (rung of the carried rows). |
| item2 | 83 | 35 | Engine Q1 applied to 34 asset rows (the recheck's 32 plus staging.rs:58 and asset_refs.rs:149): dense-component 7, system-scratch 21, kernel-internal 3, component 2, relation 1; KF-48 and KF-49 added. |
| item3 | 3 | 2 | propagation.rs:133: the delivery edge verified and recorded; form event kept, KF-24 kept rejected. |
| item4 | 296 | 161 | diagnostics narrowed: 58 numeric captures re-formed (resource-column 40, system-scratch 18), 4 per-frame text rows to system-scratch; 133 app-demo and 16 ecs-services note openings rewritten. |
| item5 | 77 | 40 | The 38 supplementary rows counted: os-owned 19, diagnostics 8, resource-column 11. |
| item5-supplementary | 38 | 38 | Rungs of the 38 counted rows. |
| item6 | 34 | 18 | One path rule: 12 boyko_log rows, AssetServer::load and save / load to resource-column; the ui boot metadata call to os-owned. |
| item7 | 108 | 106 | Rung rule step 4 widened: boyko_ecs class-K and kernel-feature rows to rung 2; the path-rule rows leave out-of-scope. |
| M1 | 1 | 1 | The 123 boyko_ui non_rows of ui-input leave the non_row total (the ui-lane is the census of record). |
| M2 | 3 | 3 | Three raw owning_entity values normalised to relation-endpoint. |
| M3 | 3 | 1 | Index text: W1-W9 -> W1-W8; `diagnostics` in the none-legal list of the reading guide. |
| M4 | 20 | 15 | Seven UiParseReport lines as ui-lane non_rows; five reflect Ident::new sites as C rows (three non_rows replaced). |
| M5 | 4 | 2 | The two gap-2 ScratchColumn-field rows marked kernel_storage_already, with the gate exception recorded. |
| M6 | 1 | 1 | The copy-pasted gap-4 note on demo render/mod.rs:62 corrected for Arc&lt;wgpu::Buffer>. |

Form changes in rev 3: item2: 1 x event -> relation; item2: 2 x resource-column -> component; item2: 7 x resource-column -> dense-component; item2: 3 x resource-column -> kernel-internal; item2: 21 x resource-column -> system-scratch; item4: 40 x diagnostics -> resource-column; item4: 22 x diagnostics -> system-scratch; item5: 8 x out-of-scope:third-party -> diagnostics; item5: 19 x out-of-scope:third-party -> out-of-scope:os-owned; item5: 10 x out-of-scope:third-party -> resource-column; item6: 15 x out-of-scope:os-owned -> resource-column; item6: 1 x resource-column -> out-of-scope:os-owned.

Index text changes in rev 3 (records with tsv_key `(index)`): the orders' ui-lane scope sentence (item 1); the `diagnostics` and out-of-scope vocabulary rows and the rev-3 vocabulary list (items 4-6); the mesh, material, asset and widget entity texts (items 1-2); KF-24's rev-3 line (item 3); the rung rule text, the rung-2 and rung-4 paragraphs and the rung table (item 7); four decisions rewritten and three added; M3's two text errors; and the asset entity text's design citation, which quoted a changelog row, not the asset table, since rev 2.

## What this ledger is not

- **Not a per-frame allocation count.** A row is a site, not a rate. The per-frame figures come from the brief's measurements (at the top of this document), and this ledger did not re-measure them. Allocation inside std's panic machinery is not counted: 122 non-test lines in ecs-schedule and 152 in codec-tools can panic, and the rest were not counted. Neither is allocation inside another group's kernel calls, nor the TokenStreams of proc macros.
- **Not a gate yet.** The gate built on it will assert three things. First, there are no `U` rows. Second, there are no NEW rows: every heap hit in non-test code is a row or a non_row, and the TSV only shrinks. Third, the per-form counts of the non-ECS forms only shrink: `kernel-internal` rows once the kernel is finished, `out-of-scope:*` rows unless re-justified, and every row whose memory still comes from std. The TSV carries no line numbers, so a line shift is not a diff.
- **Not an allocator design.** It is the input that docs/memory/ALLOCATOR-DESIGN-SPACE.md needs. Where the two disagree, the section Plan conflicts records it.
- **Not a performance claim.** No form here was benchmarked, because a timing window was running on this box. Hot-loop costs are stated only where a group reasoned them from the code, and they are marked as reasoned.
- **Not a complete picture of std/third-party allocation.** Rev 2 rows every `thread_local!` static in non-test code (31, gap 1), and since rev 3 every std env read and path conversion a group found is a counted row, but crossbeam-epoch's Local, stable-sort scratch and std path buffers outside the found call sites are not. A `#[global_allocator]` counting gate sees the rest, except the System-allocated os-key TLS cells, which only the rows count.

- **Not a census of a merged tree.** The eleven groups are joltab; ui-lane is `D:/wt/ui`; reflect-lane is `D:/wt/reflect`. Neither lane descends from joltab (both fork at 5ec1699f), and joltab has 4 commits touching boyko_ui that the ui-lane lacks. Merging them needs a re-census of the merged tree, not a union of row sets. The TSV key carries no line number, so rows survive a line shift; a row whose owner or container changes in the merge does not.

- **Not a list of migratable std heap sites, row for row.** Two rows are fields that are already kernel storage (`other:ScratchColumn`: physics `solver/warm_start.rs:214`, `narrowphase/axis_cache.rs:128`, gap 2). They are rows for their form decision (the slot-keyed PairCache), and a gate that counts remaining std heap must filter them out by container (M5).

---

*Row-count check: active rows 2350 = runtime-data-ledger.tsv data lines 2350 = sum of the group files' active rows 2350; superseded rows 131.*
