# Runtime data ledger - the rev-3 recheck, which is the rev-4 work order

- **Source:** workflow `runtime-data-ledger-rev3` (run `wf_a087d922-d51`): fixer, writer, completeness critic.
- **Status:** OPEN. Rev 3 is committed (docs/memory/RUNTIME-DATA-LEDGER.md, ledger/, runtime-data-ledger.tsv). The recheck confirmed every total and found three consistency gaps: 19 kernel rows outside rung 2, one asset datum filed under two forms, and 8 log-rotation path buffers still filed as diagnostics. Those three are the rev-4 work order.
- **Copied verbatim** on 2026-09-13; agent outputs are reproduced without edits.

---

## Rev-3 recheck (completeness critic)

**GAPS**

I re-derived every rev-3 total from the published markdown and TSV with my own parser, and all of them hold. There are 2481 rows: 2350 active and 131 superseded (122 in ui-input, 9 in render). The non_rows total 4492, which is the sum of the 13 group JSONs. No boyko_ui non_row is left in ui-input and no `src/ui` non_row in render, so M1 holds. The rungs are R2 314, R3 73, R4 161, R5 38, R6 662 and out of scope 1102. For boyko_ecs they are R2 297, R4 10, R5 6, R6 40 and out of scope 1. The forms and classes match the writer's figures exactly: 673 rows in ECS data forms, 0 class U. The crate × form, crate × class, class × form, group × rung and per-group tables have 0 mismatched cells, and so do the 13 group headers, the entity section counts and the kind/growth/addr line. The rung rule gives 0 disagreements on the 2253 rows that do not depend on kernel-feature membership. The items landed mostly as specified. What is left is 19 kernel rows outside rung 2, one asset datum carried under two forms, and 8 log-rotation path buffers still filed as diagnostics.

**Gaps**
1. **Item 7: 19 boyko_ecs rows whose destination is the kernel's own storage or a kernel feature are not in rung 2.** This contradicts the owner's order that every such row lands in rung 2.
   - **Event-channel storage, R4 (4 rows):** `joltab:crates/boyko_ecs/src/ecs/core/events/event_buffer.rs:119 "pub(crate) write_buf: UnsafeCell<Box<[MaybeUninit<E>]>>,"`, plus `:238`, `:243` and `event_dispatcher.rs:221`. Their note says "this is the kernel event channel's own storage", with one VmReservation per EventBuffer as the destination. Meanwhile the command channel's `command_queue.rs:83` and `erased_buffer.rs:109` are in R2 through KF-06. The index also contradicts itself: the rung-2 paragraph (RUNTIME-DATA-LEDGER.md:1788) puts "the event and command channel storage" in rung 2, and the rung-4 paragraph (:1792) puts "the typed event lanes" in rung 4.
   - **NonSend table, R4 (1 row):** `ecs_master.rs:127 "pub(crate) nonsend_resources: Option<Box<NonSendResources>>,"`. Its note calls it "the Resource store itself".
   - **State registrations, R6 (4 rows):** `schedule_builder.rs:92`, `:144`, `:160` and `:263`. Their note sends them to the kernel's Command records, which are the R2 row `command_queue.rs:83`. Item 7 moved every other schedule_builder B row to R2.
   - **Path index, R6 (1 row):** `asset/path_index.rs:172`. Its note says "its entries already sit in a VmColumn<PathEntry>".
   - **Re-formed D rows, R6 (9 rows):** item 4 re-formed these into kernel forms, and the "(except class D and T)" clause of rule 4 keeps them in rung 6. They are `profiling/analysis.rs:150`, `:198`, `:199`, `:200`, `:201`, `:203` and `:292`, all citing KF-01. The others are `dense_store.rs:794`, which is on KF-01's own row list, and `event_dispatcher.rs:117`.
   - **Text:** the rung-6 paragraph does not name the 18 non-diagnostics boyko_ecs rows in rung 6 (9 class B, 9 class D).
2. **Item 2: one asset datum is filed under two forms.**
   - `joltab:crates/boyko_app/src/host.rs:96 "pub(crate) retire_scratch: Vec<FreeEntry>,"` and its constructor `:271` are the backing field of `asset_refcount.rs:556`. Rev 3 re-formed :556 to dense-component with owning entity `asset`.
   - Both host.rs rows stay system-scratch / system and still say "component/dense skipped: a FreeEntry names an Assets<T> row (kind, slot), not an entity". Neither is tagged and neither mentions Q1.
   - Smaller residue: `boyko_fontbake/src/atlas.rs:126`, `:128`, `:130`, `:598`, `:611` and `:619` still justify their form with "a font is not an entity", which Q1 overturns. The form itself can stand under W4.
3. **Items 4 and 6: the path buffers log rotation builds are still `diagnostics`.**
   - The rows are `joltab:crates/boyko_log/src/sink/file.rs:197 "let oldest = std::path::PathBuf::from(format!("{path}.{keep}"));"`, plus `:202`, `:204` and `:206`, and the matching `binary.rs:381`, `:386`, `:388` and `:390`.
   - They are the paths the rotation hands to `std::fs::rename` / `metadata` at runtime. That is neither a text payload nor an error payload, so they do not fit the narrowed definition (:129).
   - Item 6 moved the std conversions of the same functions to resource-column (`file.rs:198`–`217`, `binary.rs:382`–`400`).

**What passed (checked against code)**
- **Item 1:** I ran my own lexer and regex over the 24 non-test files, which gave 93 code hits: 72 are rows, 21 are non_rows, 0 unaccounted. My regex hits every one of the 72 rows outside boyko_ui.
  - Every line the recheck named is a row: gather.rs:284/286/289, upload.rs:198/203/206/260/262/263/669, pack.rs:832/833, bindless.rs:374, the bench ×3, and the emit rows (emit/mod.rs 19, emit/shaders.rs 16, emit_ui.rs 13).
  - Each of the 9 superseded render rows points to a lane row that exists, and no joltab render row in `src/ui` is still active. All 178 item-1 evidence lines are verbatim at 615cda8f.
- **Item 2:** all 34 rows are tagged and cite Q1, and their evidence is verbatim. The design citations are verbatim: DECISIONS:23 and :35; DESIGN:513, :647, :649, :2547 and :2559.
- **Item 3:** `schedule.rs:810/817/831` and `propagation.rs:653/661` are verbatim at d11962a9. The order is apply, then drain the hook queue, then decrement successors, so the argument holds. In the working copy those lines sit at 828/835/849; the row states the commit.
- **Item 4:** 0 diagnostics rows grow per frame and 0 note openings still say kernel-internal. Three samples match the code:
  - `reduce.rs:59–63`: samples retained over a window, so resource-column;
  - `gpu_scene/mod.rs:282–306`: a capture struct, so resource-column;
  - `runner.rs:2733/2743/2822`: a per-frame String, so system-scratch.
- **Item 5:** all 38 rows are counted, with verbatim evidence: 19 os-owned boot reads, 8 diagnostics and 11 resource-column. The last includes `gpu_scene/mod.rs:6415`, now F, per-frame, R4. `third-party` is left only on the 10 boyko_demo rows.
- **Item 6:** all 17 rows are verbatim. Every remaining os-owned row is boot-only or panic machinery, and the one path rule is applied the same way to both ui rows.
- **Item 7:** all 106 moved rows are verbatim and their R2 placement is consistent with the rule.
- **Minor items:**
  - M2: 3 rows are `relation-endpoint`, and no raw values are left.
  - M3: no W1–W9 outside the change log, and :155 lists `diagnostics`.
  - M4: the 7 UiParseReport lines are non_rows, and reflect.rs:671/688/704/727/736 are rows at 0e0b4c68.
  - M5: the flag is on both rows and the exception is recorded at :145.
  - M6: the note is corrected.
- **Not rev-3 scope, noted only:**
  - 29 TSV rows differ from the markdown only by stripped line numbers or whitespace in container labels.
  - 3 labels in the index container table keep line numbers.
  - The 6 boyko_ecs scope-arena rows in R5 are defensible: the allocation belongs to the threadpool's ScopeBlock.

My scripts are in `C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/dad9a19e-5512-4bbd-9976-cb44400eeb64/scratchpad/ledger/rev3/critic3/`: `pmd.py` (parser), `t3.py` (totals), `c3.py` (item-1 census), `v3.py` (evidence, rungs, diagnostics scan), plus `v3_out.txt` and `item_rows.txt`.

---

## Rev-3 fixer report

I applied all seven work items and the six minor items, rebuilt rev 3, and published it to `D:/claude/BoykoEngine/docs/memory/`. Nothing was built or run; I used only python, git show and git diff. `verify3.py` passes on the published files with 0 errors:
- The index total, the TSV line count, the sum of the 13 group files and every index table all equal **2350** active rows.
- The TSV matches the group files' active rows exactly.
- 15238 citations are verbatim; 728 short-path ones can't be resolved by the checker, as in rev 2.
- All 18 checks come out 0 (the 7 from rev 2 and 11 new ones for rev 3).

**Totals, rev 2 → rev 3:** active rows 2244 → 2350. Superseded rows 122 → 131 (122 in ui-input, 9 in render). non_rows 4548 → 4492. Supplementary rows 38 → 0. Rungs: R2 314, R3 73, R4 161, R5 38, R6 662, out of scope 1102. There are 919 change records in `rev3/changes.json`.

**Item 1 – ui-lane census outside boyko_ui** (24 files, 145 census hits)
- **Scope:** every file the lane added plus the lines it added elsewhere. `crates/boyko_render/src/ui` is censused whole because the lane rewrote it.
- **Carried:** 7 rows moved onto their lane lines (pack.rs:801/808/853, resources.rs:464, upload.rs:517/611, bindless.rs:374).
- **New:** 65 rows:
  - render ui scratch: 14;
  - bench: 3, out-of-scope:test-only;
  - emit_ui bin: 13, out-of-scope:compile-time;
  - emit/mod.rs: 19 and emit/shaders.rs: 16, out-of-scope:compile-time.
- **Superseded:** 9 joltab render rows, including upload.rs:258/261, whose function the lane deleted.
- **non_rows:** 73 added; the 11 joltab non_rows in those files leave the total.

**Item 2 – engine Q1 (assets are entities):** 34 rows changed, the recheck's 32 plus staging.rs:58 and asset_refs.rs:149.
- **dense-component, 7:** the handle lists and orphan queues.
- **system-scratch, 21:** decode payloads, with a `Staged<A::Cpu>` component holding a span into them.
- **kernel-internal, 3:** assets.rs:204/205/207.
- **component, 2:** assets.rs:206 (the `Pinned` marker) and staging.rs:58.
- **relation, 1:** asset_refs.rs:99, as the count-only relation instead of an event.
- I added two kernel features: KF-48 (count-only relation, EK15b) and KF-49 (group release with a horizon, K6′).

**Item 3 – KF-24:** the orchestrator's hypothesis holds on joltab at d11962a9, so KF-24 stays rejected. `schedule.rs:810 "self.systems[i].system.apply(world);"` → `:817 "world.drain_deferred_hook_queue();"` → `:831 "self.executor_scratch.pred_remaining[s] -= 1;"`. The detach observer runs in the producer's apply window, so any system ordered after the producer sees the detach the same frame. propagation.rs:133 stays `event`; its note now records this evidence.

**Item 4 – diagnostics split**
- **Numeric captures:** 58 rows re-formed (40 resource-column, 18 system-scratch). The recheck's figure of 81 counted every non-String container, which includes text payloads such as `Vec<String>` and `Vec<Problem>`; those stay `diagnostics`.
- **Per-frame rows:** the 4 runner.rs strings become system-scratch; the 6 reduce.rs rows become resource-column.
- **Notes:** 149 openings rewritten (133 app-demo, 16 ecs-services).

**Item 5 – supplementary rows:** all 38 are counted rows now.
- 19 boot env reads: out-of-scope:os-owned.
- 8 diagnostic env reads: diagnostics.
- 10 dump and sink path conversions: resource-column, under the item 6 rule.
- gpu_scene/mod.rs:6415, the per-frame env read: resource-column, read once at boot. Its note says so.

**Item 6 – one path rule:** 15 rows moved to resource-column: 12 boyko_log rows, server.rs:127, save.rs:717 and load.rs:312. The ui-lane `plugin.rs:132` call runs only at boot, so it became out-of-scope:os-owned; `reload/system.rs:123` is unchanged. One difference from your brief: the two log-sink opens (file.rs:131, binary.rs:467) are not boot-only. `request.rs:212 "let _ = crate::sink::file::open(cap_bytes);"` re-opens the sink at runtime, so they fall under the rule too.

**Item 7 – rungs:** rule step 4 now puts boyko_ecs rows in rung 2 if they are kernel-internal, class K, or a row of an active kernel feature. Class D and T rows are excluded; without that, the path-conversion row at server.rs:127 got into rung 2 only because it shares a line with a kernel-feature row.
- 61 rows moved R4 → R2 and 29 moved R6 → R2, including all 11 class-K rows. The rung-4 text is corrected.
- 15 rows left out-of-scope for R4 and 1 moved R4 → out-of-scope (the item 6 changes).
- boyko_ecs now sits at R2 297, R6 40, R4 10, R5 6, out of scope 1.

**Minor items**
- **M1:** the non_row total drops the 123 boyko_ui non_rows of ui-input. With the other changes it is 4492.
- **M2:** the 3 raw owning_entity values are now `relation-endpoint`.
- **M3:** both W1-W9 mentions now read W1-W8, and the reading guide's none-legal list now includes `diagnostics`. I also fixed one extra error from rev 2: the asset entity text cited ENGINE-RUNTIME-ECS-DESIGN.md:1656, a changelog row that has nothing to do with asset kinds.
- **M4:** 7 UiParseReport non_rows added. `Ident::new` became C rows at reflect.rs:671 and :688, and also at :704, :727 and :736, where the rows replace 3 non_rows.
- **M5:** the two ScratchColumn-field rows are flagged `kernel_storage_already`, and the index records that a migration gate must skip them.
- **M6:** the demo render/mod.rs:62 note now describes the `Arc<wgpu::Buffer>` correctly.

Files are in `C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/dad9a19e-5512-4bbd-9976-cb44400eeb64/scratchpad/ledger/rev3/`:
- changes.json
- item1/ui_ext_census.py
- item1/ui_ext_census.json
- synth3/build3.py
- synth3/text3.py
- synth3/kfcalc.py
- synth3/qcite.py
- synth3/make_gen3.py
- synth3/gen3.py
- synth3/verify3.py
- synth3/final.json

---

## Rev-3 writer report

I rebuilt rev 3 from the rev-3 scripts and published it to `D:/claude/BoykoEngine/docs/memory/`. Every total and every index matrix matches the rows as written: **2350 active rows**, with 0 errors from the rev-3 checker and 0 from my own. Nothing was built or run beyond python and git; I made no edits to the code trees and did not touch the rev-2 files.

**How I rebuilt it** (my copies are in `rev3/writer3/`)
- **Fixer re-run:** `build3.py` rebuilt `final.json` byte-identical to the fixer's. `changes.json` holds the same 919 records, but 12 are in a different order. Those 12 are runner.rs item-4 records; the order is not stable between runs.
- **Renderer re-run:** `make_gen3.py` and `gen3.py` produced output that differs from the fixer's publish in only two ways:
  - **Date:** the fixer's header said `2026-09-13`; it now says `2026-09-11`, as the brief asked.
  - **Tie order:** two "Current forms" lines list tied counts in a different order.
- **ui-lane render census:** it goes into `ui-lane.md`, which now has 196 rows. No new group file, so there are still 13.

**Checks, run on the published files**
- **Row totals:** the index total, the TSV data lines, the sum of the 13 files' active rows, the per-group table and each matrix total are all 2350.
  - All rows: 2481, of which 131 are superseded (122 in ui-input, 9 in render). Supplementary rows: 0.
  - non_rows: 4492 in the index, the same as the group JSONs.
- **TSV:** 12 columns, sorted, no line numbers. Its rows equal the active rows as a multiset. 504 lines are the same row at different source lines (rev 2 had 471).
- **Citations:** 15238 check out verbatim; 728 use short paths the checker cannot resolve.
- **The 18 invariants in `verify3.py`:** all 0.
- **My own verifier** (`writer3/indep_verify.py`) reads only the written markdown and TSV. It rebuilds every index matrix and count from the parsed rows and gets 0 mismatches.
  - It covers crate × form, crate × class, class × form, entity × form, group × rung, the per-group table, containers, kind/growth/addr_cached, entity section counts, and the rung paragraphs, check line and history table.
  - It also checks each group header and footer.
  - It recomputes each row's rung from the 7-step rule: 0 disagreements. Kernel-feature membership is the one input it takes from `final.json`. That membership decides 79 rows.
  - To show it can fail, I planted a wrong matrix cell and a wrong row rung in a copy. It reported 13 errors.

**Numbers**

| rung | R2 | R3 | R4 | R5 | R6 | out of scope |
|---|---|---|---|---|---|---|
| all rows | 314 | 73 | 161 | 38 | 662 | 1102 |
| boyko_ecs | 297 | – | 10 | 6 | 40 | 1 |

- **Forms:** compile-time 1021, system-scratch 370, diagnostics 301, kernel-internal 264, resource-column 155, dense-component 61, test-only 40, component 32, relation 31, os-owned 31, event 16, scope-arena 10, third-party 10, enable-state 8.
- **Classes:** C 1061, B 498, D 376, F 182, K 86, R 62, T 44, E 25, S 11, X 5, U 0.
- **Change log:** the index has a rev 2 → rev 3 table with one line per item (items 1–7, the carried and supplementary sub-items, M1–M6), plus the form changes.

**Worth knowing:** the item-3 citations (`schedule.rs:810/817/831`) hold at commit `d11962a9`, not in the joltab working copy. There, `schedule.rs` has uncommitted edits that shift those lines by about 17. At the commit, :810 is `self.systems[i].system.apply(world);`, :817 is `world.drain_deferred_hook_queue();`, and :831 is `self.executor_scratch.pred_remaining[s] -= 1;`. This matches what the index's tree table already says about the working copy.

Files are in `C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/dad9a19e-5512-4bbd-9976-cb44400eeb64/scratchpad/ledger/rev3/writer3/`:
- indep_verify.py
- verify3_published.log
- indep_verify_published.log

Published files (uncommitted):
- `D:/claude/BoykoEngine/docs/memory/RUNTIME-DATA-LEDGER.md`
- `D:/claude/BoykoEngine/docs/memory/ledger/*.md` (13 files)
- `D:/claude/BoykoEngine/docs/memory/runtime-data-ledger.tsv`
