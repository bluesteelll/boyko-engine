# msvc lane citation repair - the pass-7 verifier report, which is the pass-8 work order

- **Source:** workflow `msvc-citation-repair-pass7-final` (run `wf_3f73c046-da3`), verifier
- **Status:** OPEN. Passes 1-7 are applied in D:/wt/msvc (register: docs/OPEN-QUESTIONS.md, Pass 1-7 sections). Pass 8 (run `wf_86934723-4ac`) was stopped before it wrote anything.
- **Copied verbatim** on 2026-09-11 at the checkpoint the owner asked for; agent outputs are reproduced without edits.

---

## Pass-7 verifier report

NOT SOUND

**Quotable line:** Pass 7 did everything it was asked to do, but it applied the orchestrator's dating rule only to the lines pass 6 restored, not to the debt table. Two critique/appendix lead-ins that the rule clearly covers were missed, so the debt is 153 numbers on 73 lines, not 170 on 80.

**Blocking findings**

1. **Item 6 and item 7: `KE16-DESIGN-B4.md:312` is an as-of for the whole round-2 critique, and nobody read it.**
   - The line reads "Reviewed 2026-09-07 against `d647d930`. Every site below was re-verified by hand." It sits directly under `# ⚠ ROUND-2 CRITIQUE` (`:310`), and the critique runs from there to the end of the file (`:463`). It was written at `7fdd738e`, like the lines it covers. The document-wide header is `:3`, a different line.
   - It covers five debt lines, 9 numbers: `:378`, `:385`, `:421`, `:456` and `:457`.
   - Every original number holds at `d647d930`. Only B4 changed between `d647d930` and `7fdd738e`, so they hold at the writing commit too:
     - `KE16-RESULTS.md:12` is "B0 BY CONSTRUCTION", `:531` is "axis A closes on `a3`", and `:1158-1163` is the `a1` worker-route block (`top_lane` at 1160);
     - `miri_scope.rs:567-572` is the `cfg_attr`;
     - `loom_pool.rs:340` is the steal-transport fence, and `:233-240` is the consumer-fence note.
   - This is the same shape of lead-in as `VG-R3-P2:1426` and `VG-R3-P3:2712`/`:2890`, which were counted as dating clauses. Under the rule these five lines are dated records, not debt.
   - So these register sentences are false: "Its one as-of, at `:408`" (OQ 5908 row, 6156, 6383-6387), "F3's debt decision stands", and (7)'s row for `B4:456`-`457` ("dates its own claim only").
   - Restoring them (575→531, 1202-1207→1158-1163, 572-577→567-572, 366→340, 259-266→233-240) is a document edit, which this pass's scope does not allow. That decision is yours.
2. **`VB-SV0-SDF-SHADOW-PLAN.md:1627` and `:1628` (8 numbers, counted as "refused continuations") are dated records.**
   - They are in the same paragraph as `:1625`, which pass 6 restored, and under the same §9 lead-in, `:1542`: "Every line below was opened while writing **this revision**".
   - Blame puts `:1542`, `:1627` and `:1628` all at `62731d91`. At that commit `PINS.toml` has `[vb_both]` at 313-343 (empty list at 322, note at 326-328) and `[vb_sdf_only]` at 345-377 (empty at 355). All 8 numbers hold.
   - Both lines are byte-identical at `e6115223`, at `97bcf826` and in the tree. They are dated records kept as written, the same class as F1's `:2924`. Only the register needs to change.
3. **The final accounting follows from 1 and 2.**
   - Debt should be **153 numbers on 73 lines**: `device.rs` 34/50, `PINS.toml` 22/64, `loom_pool.rs` 17/39, `miri_scope.rs` 0, `KE16-RESULTS.md` 0.
   - By class: refused 21→13, F3 4→0, `miri_scope.rs` 2→0, `loom_pool.rs` 35→32.
   - If you overrule the extension to `KE16-DESIGN.md:359`, `:360` and `KE16-DESIGN-SPACE.md:554`, it is 149 on 70.
   - The durable-fix opt-out list, "37 restored lines", also leaves out F1's `:2924`, the two SV0 lines, and the five B4 lines once they are restored. None of them carries `<!-- doc-anchor-ignore -->` today.

**Minor**

4. OQ 6156 and the B4:378 row say the `:408` as-of scopes the census anchors "on its own line". The anchors are on `:407`; the as-of sentence is on `:408`.
5. Two superseded figures are corrected by a bracket but not struck: OQ 6171's "50 were true at `e6115223`" and OQ 5723's "147 of the 152 were". In the same list, "95", "3" and "5" were struck.

**Verified as landed**

- **Item 1:**
  - `device.rs:3158` is the shadow-denoise `eprintln!(` at `053f6c9f`, `303a7a92` and `d02f74a3`.
  - At `b30fa810` it is already inside the DDGI reporter; its parent has none of the three sites.
  - At `e6115223` it is `W2102,` in `report_ddgi_storage_unsupported`, with shadow-denoise at 3174/3179.
  - At HEAD, 3176 is the DDGI `W2102,` and shadow-denoise is at 3192/3197.
  - The five citing lines carry 3176 and blame to the stated commits. The rows and counts are right: 154/73 after (1), 140 of 152, and 90+50+2+8+2=152.
- **Item 2:** my own scan of all tracked non-binary files gives 237 link-form hits: the repairer's 235 plus pass 7's two quotations at OQ 6311 and 6348, both marked.
  - Lane-file hits: 6, all in MESHLET. Only `:2517` shifts (372→390, 417→435). The other four map to themselves, range ends included.
  - `#L` form: 237 hits, none naming a lane file. `](path:N)` form: 0.
  - At `6d1af3a9`, 372 and 417 are `test_name` of `[vb_both_sdf]` and `crate` of `[vb_both_sdf_tex]`; at HEAD, content lands on 664/709.
  - The `~` waiver is at `internal_docs_anchors.rs:737`, and MESHLET is the fourth entry in `GATED_DOCS`.
- **Item 3:** your F1 decision is recorded, with both pointers. The two critique as-ofs (`:2712`, `:2890`) exist as quoted.
- **Item 4:** 8 of the 9 quotation lines are byte-identical to `97bcf826`; only 1010 differs. `:1548` reads 259/240/258/266, and those are the four stated lines at `9e80cd4e`.
- **Item 5:** the two REPL lines are `P2:1011` and `P3:988`. At `b1725b32`, line 3732 is `#[cfg(test)]` and 3733 is `mod tests {`.
- **Item 7:** the counts are right for the 35 pass-6 lines: 29 dated (105 numbers), 6 pre-lane rot (14), and 21 of the 24 found lines dated.
  - The six pre-lane lines are byte-identical to `97bcf826`.
  - The 14 numbers hold at `778739f0` and none holds at `e6115223`. At `778739f0`, `loom_pool.rs` has 387 lines. It later grew to 1640 and was cut to 644 at `67563d3b`.
- **Final accounting:**
  - Lane shifts, 46 lines / 91 numbers: `lane90.py` rerun gives 90, 0 bad (78 exact, 2 REPL, 10 config) on 45 lines, plus `PARTICLES:183`.
  - Quotations 9/16, dated records 29/105, the 37/121 union, and F1 1/2 all check.
  - Parsing the table with the register's own reading rule gives 80/170 and the stated per-target and per-class splits. Findings 1-3 are about how lines are classified, not about the arithmetic.
- **Hygiene:**
  - Only `OPEN-QUESTIONS.md` changed after the pass-6 state: every other file's mtime is at or before 07:09:44; 539 lines v7 recorded across 59 files are unchanged; the 35 pass-6 restores still hold their text.
  - The snapshot matches v7's 19 recorded OQ lines.
  - The OQ diff is 36 lines, all insertion or strike-through, plus the three rows at 5932-5934 and the append from 6247. `lines 5908-5909` still holds.
  - The first 5165 lines differ from `97bcf826` only at 1954, 2754, 4189, 4209 and 4989.
  - CRLF, LF and CR counts are each 6529. There are 28 ` M` files and nothing untracked. `docs/ru` is untouched. HEAD is still `97bcf826`.
  - No unmarked citation-shaped line remains in 5166-6529.

My scripts (read-only) are in `C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/dad9a19e-5512-4bbd-9976-cb44400eeb64/scratchpad/v8/`: `ins.py` (with `ins.txt`), `debt.py` (with `debt_rows.json`), `link.py`, `leadin.py` and `lane90_repl.py`.
