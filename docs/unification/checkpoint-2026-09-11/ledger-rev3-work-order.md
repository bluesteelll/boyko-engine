# Runtime data ledger - the rev-2 recheck, which is the rev-3 work order

- **Source:** workflow `runtime-data-ledger-rev2` (run `wf_7c3b8f78-6af`), recheck and writer reports
- **Status:** OPEN. Rev 2 is written (docs/memory/RUNTIME-DATA-LEDGER.md + ledger/ + .tsv). Rev 3 (run `wf_a087d922-d51`) was stopped in its first phase at the checkpoint; nothing of it was written to the repository.
- **Copied verbatim** on 2026-09-11 at the checkpoint the owner asked for; agent outputs are reproduced without edits.

---

## Recheck and writer reports

# Ledger rev-2 recheck (verbatim)

**GAPS**

Rev 2 is arithmetically sound. I parsed the 13 group files, the TSV and the index with my own scripts and got the writer's numbers: 2366 rows, 2244 active, 122 superseded, 38 supplementary rows not counted, and 4548 non_rows (which equals the sum of the group JSONs). Every index matrix reproduces with 0 mismatched cells. That covers crate × form, crate × class, class × form, per group, group × rung, kind/growth/addr_cached, the entity counts, and all 13 group headers. The rungs are R2 223, R3 73, R4 202, R5 38, R6 667 and out of scope 1041. Recomputing each row's rung from the 7-step rule gives 0 disagreements. The TSV equals the active rows as a multiset on 9 key fields and on container<elem>. There are 0 U rows, 0 forms outside the vocabulary, and the 358 D rows are 355 `diagnostics`, 2 `os-owned` and 1 `resource-column`.

The writer's call on superseding all 122 boyko_ui rows is correct. They map one-to-one onto 122 distinct ui-lane rows with the same owner, container, class and kind. The evidence is identical except `dispatch.rs:92` (`{name:?}` against `{other:?}`), and the two unmatched lane rows are the two new ones. Joltab-only boyko_ui code (vocab.rs and the rest) has no unrowed site where heap memory is created.

What does not hold is the delta's own completeness: one lane's scope, the propagation of engine Q1, one rejected kernel feature, how the diagnostics form fits its rows, and the supplementary rows.

**What passed (checked against code)**
- **Lane censuses (my own lexer and regex).**
  - ui-lane, 45 files: 524 hit lines. Every line that creates heap memory is a row or a non_row, except 7 `UiParseReport` lines (M4 below).
  - reflect-lane: 43 code hits, all accounted for.
  - `thread_local!` and `Box::leak` in non-test lane code: 0 in both lanes.
  - joltab: 31 non-test `thread_local!` statics, all 31 rowed (gap 1 confirmed). All 19 non-test `Box::leak` sites are a row or a non_row.
- **Gap samples re-derived from code.** At least 3 per gap: tls.rs:169, sync_out.rs:75 and scope.rs:31; warm_start.rs:214 and axis_cache.rs:128; soft/component.rs:71, :330, :511 and :540; demo render/mod.rs:62, gpu_scene/mod.rs:2206 and upload.rs:261; tags.rs:195, schedule_builder.rs:707 and bundle_column_cache.rs:416; reduce.rs:61, artifact.rs:197 and vg_census_dump.rs:336; propagation.rs:120, :124 and :404 plus pick.rs:110 and :117 (both systems are `fn(&mut EcsMaster)`); window.rs:122, window.rs:230 and queue.rs:35; artifact.rs:854 and contrast.rs:136 (not cfg-gated, while all 37 remaining test-only rows are); archetype_registry.rs:419; prof_decode main.rs:182. The citations in the defect sections are verbatim.
- **Citations.** 3718 resolvable ones are verbatim at their stated line; 0 were stale. That includes PHYSICS-ECS-UNIFICATION-DESIGN.md, which was rewritten at 07:42:16, after the 07:31 publish.

**Work items**
1. **The ui-lane census stopped at `crates/boyko_ui/src`.** The lane also changes non-test code elsewhere, and those sites are neither rows nor non_rows anywhere in rev 2:
   - boyko_render: `D:/wt/ui:crates/boyko_render/src/ui/gather.rs:284 "pub roots: Vec<Entity>,"` (also :286 and :289), `upload.rs:198 "staging: Box<[UiInstance]>,"`, `:203 "node_buf: Vec<UiNode>,"`, `:206 "keys: Vec<(u32, u32)>,"` (also :260, :262, :263 and :669), `pack.rs:832/833`, and `bindless.rs:374`.
   - `benches/ui_pack_sort.rs` ×3.
   - boyko_shaderdsl: emit_ui.rs 15, emit/mod.rs 19, emit/shaders.rs 16 (these would be C rows).

   The lane's own note (ui-lane.md:57) says "a render-lane census is owed". The index drops this, and main:docs/memory/RUNTIME-DATA-LEDGER.md:70 calls the ui-lane "its rev-2 census". Fix: census the lane's changes outside boyko_ui by the reflect-lane's scope rule (every added file plus the added hunks, in every crate). Then supersede the stale joltab render rows for boyko_render/src/ui.
2. **Engine Q1 (assets are entities) was adopted but not applied.** 32 active rows still skip `component` or `relation` because "an asset row is not an entity", and none of their notes mentions Q1:
   - render 19: gpu_upload.rs:216, mesh_assets.rs:582/712, texture.rs:709/928, asset_refcount.rs:556, and the payload rows in glb, obj, png_texture, mesh_data and texture_data;
   - codec-tools 8: atlas.rs:116 and png.rs ×7;
   - ecs-services 4: assets.rs:204–207;
   - asset_refs.rs:99.

   The index decision "Asset stores" (RUNTIME-DATA-LEDGER.md:1776) says asset values become K3 group columns. Yet its 5 rows stay `resource-column` with no rev-2 block. asset_refs.rs:99 stays `event`, although main:docs/unification/ENGINE-RUNTIME-ECS-DECISIONS.md:35 says "Refcounting becomes a count-only relation (EK15b)", and relation comes before event in the first-fit order.
3. **Rejecting KF-24 (writer change W5) leaves one row without the feature it needs.** `joltab:crates/boyko_scene/src/propagation.rs:133 "detached: Vec<Entity>,"` is an `event` whose note requires same-frame delivery: "next-frame visibility would leave a stale GlobalTransform for one frame". The index says KF-24 is needed by no row ("none needs it now", RUNTIME-DATA-LEDGER.md:1248), which is false for this row. W5's reason (UI messages become triggers, engine ED9) does not cover a queue from an observer to a system. Re-decide it by performance.
4. **The `diagnostics` form does not fit many of its rows.**
   - 81 of the 355 rows are numeric capture data, not text: `joltab:crates/boyko_app/src/profiling/reduce.rs:61 "begin_off_ns: Vec<f64>,"`, gpu_scene/mod.rs:282–306 `Vec<u32>`, vg_census.rs histograms, analysis.rs bit matrices.
   - 10 of the rows grow every frame (reduce.rs ×6, runner.rs:2733/2743/2762/2822).
   - The definition at RUNTIME-DATA-LEDGER.md:127 ("text and error payloads") fits none of these, and their notes prescribe ScratchColumn lanes. Widen the definition or re-form the rows.
   - 149 diagnostics rows still open their ECS note with "kernel-internal = …", and 16 ecs-services rows still say "CHOSEN kernel-internal". Rewrite those openings.
5. **The supplementary rows are not "closed".** RUNTIME-DATA-LEDGER.md:1675/1681 says they are. But the 38 app-demo std-internal rows remain uncounted:
   - 37 are `out-of-scope:third-party`, which the rev-2 vocabulary forbids for std internals; W6 fixed only save.rs:717 and load.rs:312.
   - 18 are class D outside `diagnostics`.
   - One runs every frame, `joltab:crates/boyko_app/src/gpu_scene/mod.rs:6415 "let vb_force_classified = std::env::var(\"BOYKO_VB_FORCE_CLASSIFIED\").is_ok();"`. Neither the 2244 count nor the "TSV only shrinks" gate can see it.
6. **The T-row decision is applied two ways.** The two ui-lane path-conversion rows (for example `D:/wt/ui:crates/boyko_ui/src/reload/system.rs:123 "let meta = std::fs::metadata(path).ok()?;"`) became `resource-column`. The same mechanism stays `out-of-scope:os-owned` at runtime:
   - log rotation in boyko_log: `joltab:crates/boyko_log/src/sink/file.rs:198 "let lost = std::fs::metadata(&oldest).map(|m| m.len()).unwrap_or(0);"`, plus the rest of file.rs and binary.rs ×6;
   - asset load in boyko_ecs: `joltab:crates/boyko_ecs/src/ecs/core/asset/server.rs:127 "let handle = match std::fs::read(path) {"`;
   - save and load in boyko_serialize: save.rs:717 and load.rs:312.

   This contradicts RUNTIME-DATA-LEDGER.md:1775, which moves every path conversion reachable after steady state to the in-house UTF-16 FFI.
7. **The rung rule is exact, but its reach is narrower than gap 5's stated reason** ("the owner ordered the kernel finished first").
   - 11 class-K boyko_ecs rows sit at R4, after physics: schedule.rs:122/161/172, entity_store.rs:120/123/125, system_meta.rs:140, and assets.rs:204–207.
   - 70 boyko_ecs rows are in R4, although the rung-4 text (RUNTIME-DATA-LEDGER.md:1731) says "the other crates' rows".
   - 38 non-diagnostics boyko_ecs rows are in R6.

   Either extend rule 4 to those rows or correct the text.

**Minor**
- **M1. The non_row total double-counts boyko_ui.** ui-input's JSON holds 123 boyko_ui non_rows, 112 of them the same lines as the lane's 123. Consistent with the superseding rule, the total is 4425, not 4548.
- **M2. Three owning_entity values are not in the vocabulary.** They are raw text in the TSV and the group files ("parent-entity (any kind)" and two variants). Normalise them to `relation-endpoint`.
- **M3. Three small text errors in the index.** Lines 20 and 2082 say W1–W9, but the change log lists W1–W8. Line 145 leaves `diagnostics` out of the forms that may have owning_entity `none`; line 139 includes it.
- **M4. Seven ui-lane lines are neither a row nor a non_row.** They are the `UiParseReport` lines report.rs:19/38, state.rs:126/151, system.rs:176, ast.rs:116 and parser.rs:50. The ui census regex lacked the engine's own heap-type names (the reflect census had them). None of these lines allocates, so they belong as non_rows. Separately, reflect.rs:671/688 (`Ident::new`) are unrowed while `format_ident!` at 418/498/522 are C rows.
- **M5. The gap-2 rows are not heap sites.** They are `ScratchColumn` fields, which the row definition excludes. Record the exception, or the future gate will count them as rows that can never migrate.
- **M6. A copy-pasted gap-4 note.** The rationale "this Vec is engine heap…" was applied to `Arc<wgpu::Buffer>` at demo render/mod.rs:62.

Nothing was built or run; I used python, git show and git diff only. My scripts are in `C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/dad9a19e-5512-4bbd-9976-cb44400eeb64/scratchpad/ledger/rev2/critic2b/` (`parse_md.py`, `consist.py`, `lex2.py`, `census2.py`, `account.py`, `triage.py`, `calls.py`, `retfns.py`, `tls3.py`, `cites2.py`, `cites_docs.py`, with their JSON outputs). I did not use `rev2/critic2/`: another critic session had written files there between 07:32 and 07:45, so I left them alone.

# Ledger rev-2 writer report (verbatim)

Rev 2 of the ledger is written and split, and every check passes: the index total, the TSV data-line count and the sum of per-group active rows are all **2244**.

**The numbers**
- **Rows:** 2366 in total, 2244 active and 122 SUPERSEDED. All 122 superseded rows are boyko_ui rows in `ui-input.md`, each with a pointer to its ui-lane row.
- **Where the 2244 come from:** the 11 groups hold 2223 rows after the gaps, minus the 122 superseded, plus 124 from ui-lane and 19 from reflect-lane.
- **Rungs:** R2 223, R3 73, R4 202, R5 38, R6 667, out of scope 1041.
- **Kernel features:** 47. That is the 43 from rev 1, KF-44 to KF-46 from the gaps, and KF-47, the reflect lane's by-id seam. The ui-lane added none.
- **Other invariants:** 0 U rows, 0 D rows left in kernel-internal, and 0 forms outside the vocabulary.

**One call you should check: all 122 boyko_ui rows are superseded, not the 27 the lane report named.** The lane carries every one of the 122, so counting both sets would count each site twice, and the "exactly once in the TSV" check could not hold. The lane's 27 are the rows whose content changed; the other 95 are superseded because they are the same sites.

**What I changed beyond merging the inputs** (all logged in `rev2/synth2/writer_changes.json`):
- **W1:** The ui-lane was built from rev-1 rows, so it lacked the gap changes. I merged them in: 17 D rows go to `diagnostics`, and the gap-7 notes were three-way merged with no conflicts.
- **W2:** The 5 reflect-lane D rows go to `diagnostics`, following gap 6.
- **W3:** Physics Q3 applied to the ledger. The IslandSleep sleep latch and its counter (4 rows) become dense-component `BodyGate`, and I withdrew KF-16 and KF-18.
- **W4:** Engine decision Q1 (assets are entities) applied to fonts and sprite sheets. `FontTable::fonts` and `UiSheetTable::sheets` become components on a new `asset` entity, which overturns the lane's UL-D3 for those two rows.
- **W5:** Engine design ED9 makes hover a Trigger, so I rejected KF-24.
- **W6:** Two codec-tools std path-conversion rows were tagged third-party; they are re-tagged `out-of-scope:os-owned`.
- **W7:** The physics decisions file line 89 (the RunCtx trampoline) was struck out after gap 1 cited it. I replaced 19 citations and corrected KF-45's route.
- **W8:** `ALLOCATOR-DESIGN-SPACE.md` was rewritten at 05:51, so 1614 citations were re-anchored by +21 lines. Each quote was re-found in the current file.
- **Other decisions:**
  - Every rev-1 open item is decided in the index section "Decisions", each with the gate that would overturn it.
  - Rev 1 proposed intrusive links for KF-11; I follow the engine design's K7 spans (EK15c).
  - KF-37 is superseded by the engine's K6′, and KF-38 is withdrawn.

**Verification** (`verify2.py`, run on the written files):
- The TSV, read as a multiset, equals the active rows parsed from the 13 group files. It has 12 columns, is sorted, and carries no line numbers.
- 471 TSV lines are identical rows at different source lines, which is expected because the TSV has no line column. Rev 1 had 469.
- Citations: 14608 check out verbatim.
  - 728 use short paths my checker cannot resolve; the group builders verified those, I did not.
  - 110 hold only at commit `ca582e72`, because the joltab working copy has uncommitted edits.
- Largest file is `codec-tools.md` at 1.05 MB; the index is 261 KB.

The synth scripts' rev-1 descriptions are superseded by the new ones in `rev2/synth2/`. Nothing was built or run beyond python and git.

Files are in `D:/claude/BoykoEngine/docs/memory/`:
- `RUNTIME-DATA-LEDGER.md` (the index)
- `ledger/*.md` (13 files: the 11 groups, `ui-lane.md`, `reflect-lane.md`)
- `runtime-data-ledger.tsv`

The scripts and logs are in `C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/dad9a19e-5512-4bbd-9976-cb44400eeb64/scratchpad/ledger/rev2/`:
- `synth2/`: `build_rev2.py`, `gen2.py`, `text2.py`, `verify2.py`, `cites.py`, `final.json`, `writer_changes.json`, `verify_final.log`
- rev-1 backups: `rev1-RUNTIME-DATA-LEDGER.md.bak`, `rev1-runtime-data-ledger.tsv.bak`
