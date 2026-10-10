# Unified system plan — 04 Branch integration (rev 6)

Tree tags (`[J]`, `[M]`, `[G]`) and the citation verification record are in
[00 Overview](UNIFIED-SYSTEM-PLAN-00-OVERVIEW.md).

## 1. The fleet

Heads were read from `[M].git` on 2026-09-17. The architect's "Ahead / .rs" and merge-base facts
came from [G] §2.3 (`[G]:396-418`, diffed against `d552be05`). The writer re-read every row the
same day with read-only git (`rev-list --count d552be05..<tip>`, `diff --name-only
d552be05...<tip> -- '*.rs'`, `merge-base`, `merge-base --is-ancestor <tip> d552be05`,
`git cherry`, `status --short`); the numbers below are those re-reads.

| Worktree | Branch @ head | Content | Ahead / .rs (merge base) | State → target |
|---|---|---|---|---|
| `D:/claude/BoykoEngine` | `feat/multi-paradigm-render` @ `b716a5dc` (`.git/refs/heads/feat/multi-paradigm-render`, 2026-09-17) | docs, plus the owner's committed work (Q-5): `5e86fe2d`, `f37650a6`, `f20bdafe`, `69cf3f79`, `5aef7b39`, `b0bff31f`, `b716a5dc`. Not committed: `.claude/settings.local.json` and the two model archives under `assets/models/` | The rev-5 "14 / 0" is stale, because the owner's commits carry `.rs` files. The writer re-reads the count (the architect has no shell). *Writer (00 §9 V-64): 22 / 16 (`97c504c8`); `ac86fc38` (the plan and allocator rev 2) also landed after `8a78ef6d`.* | O1 done; O2; then receives the trunk |
| `D:/wt/joltab` | `merge/ke16-into-ecsnative` @ `d552be05` | kernel + physics + render superset; A1 in flight | base | **partly pushed:** the upstream is at `b74f7ee8`, which carries the reachability-census red (session record); `7c327121`, `fc7eb127` and `d552be05` are ahead of that upstream, and the push waits on F1. Of the three, only the merge commit `d552be05` exists nowhere else: `7c327121` and `fc7eb127` are also on `origin/fix/rejected-gpu-upload-leak` (writer check, 00 §9 V-28). Becomes the trunk, and its worktree becomes the **trunk worktree**: after A8 it carries merges and full gate runs only (02 §4.1). every rung from Phase B on takes a code-pool worktree, and the pool opens at A8 (02 §4.1) |
| `D:/wt/uploadleak` | `fix/pool-panic-and-reservoir-loom` @ `d552be05` | A2 in flight: 14 modified tracked files, 9 untracked entries | 0 committed | → trunk (A2) |
| (no worktree) | `fix/rejected-gpu-upload-leak` @ `fc7eb127` | defect B + VB teardown | contained in `d552be05` (verified); `d552be05` is local-only (row above), so `origin/fix/rejected-gpu-upload-leak` is the only pushed ref that holds these two commits | prune **only after** `merge/ke16-into-ecsnative` is pushed past `d552be05` |
| `D:/wt/refactor` | `refactor/split-oversized-files` @ `d552be05` | census + tools | 0 | parked until F4 (Q-4); rebased onto the trunk at F4; waves per 02 §5 |
| `D:/wt/merge` | `merge/ke16-into-render` @ `d2c8c646` | KE16 into render: 16 docs + `tests/gaia_ruled_vs_open_census.rs` | 9 / 1 (`02325b01`) | O2: a true merge with main @ `b716a5dc` (Q-5; §3) |
| `D:/wt/kernel` | `fix/ke13-ke14` @ `1aaadfdc` | KE13 / KE14 | 1 / 25 (`b6c41237`) | A3 |
| `D:/wt/reflect` | `feat/reflection` @ `0e0b4c68` | reflect lane, KF-47 | 20 / 82 (`5ec1699f`); 2 commits ahead of its upstream | A6 |
| `D:/wt/ui` | `feat/ui-advanced` @ `615cda8f` | UI lane | 16 / 69 (`5ec1699f`); 2 commits ahead of its upstream | A7 |
| `D:/wt/ddgi` | `fix/ddgi-host-hook` @ `230585d0` | DDGI host hook | 7 / 17 (`97c504c8`) | A5 (GPU gate is the owner's) |
| `D:/wt/assets` | `fix/asset-validate-prereqs` @ `419aaf2d` | asset prerequisites | 5 / 8 (`97c504c8`) | A5 |
| `D:/wt/golden` | `feat/golden-edsl-p0` @ `a8fc2e2a` | SSAO oracle from eDSL | 5 / 4 (`97c504c8`) | A5 (steps 4–5 are the owner's) |
| `D:/wt/msvc` | `chore/msvc-host` @ `a36ceaa4` (pushed: `origin/chore/msvc-host` is `a36ceaa4`) | msvc recipes + citation repair | 3 / 17 (`e6115223`); `boyko_rhi_vulkan/src/device.rs` is also edited by the owner's committed work (Q-5) and is resolved by hand at A8 (§2 step 1a) | **AH** (§2 step 1a) |
| `D:/wt/census` | `fix/census-post-lto-object` @ `27ac8904` (pushed per `packed-refs`; its worktree reflog ends at that commit; cleanliness not checked; *writer: `git status --short` there is empty, 2026-09-17*). It fixes the two msvc census reds by reading the post-LTO object (`[C]crates/profile_fixture/tests/profile_axis_census.rs:46-97`) | symbol census on post-LTO objects | 1 / 3 (`02325b01`); no census file | **AH**, first (§2 step 1a) |
| `D:/wt/docgates` | `chore/doc-gates` @ `add0c21b` | doc gates | 10 / 3 (`02325b01`); no census file | A5 |
| `D:/wt/register` | `docs/ab-register-sync` @ `4559b6a3` | AB index + RULED-vs-OPEN census | 5 / 1 (`97c504c8`); no census file | A5: take only `tests/gaia_ruled_vs_open_census.rs` after O2 (session record) |
| `D:/wt/ecsnative` | `feat/ecs-native-storage` @ `ad0ebea4` (pushed) | superseded as a storage lane; 2 untracked census copies (`alloc_frame_{attribution,census}.rs`) | **1 / 1, NOT contained:** `ad0ebea4` adds `tests/physics_vec_side_store_census.rs` (1,172 lines), which `[J]` does not have and physics R0 restores (`PHYSICS-ECS-UNIFICATION-DESIGN.md:789`) | **keep** until B4 takes that file; review the 2 untracked census copies; prune after |
| `D:/wt/threadpool` | `feat/threadpool-ke16` @ `5550a3da` | KE16 | contained (verified); clean | prune |
| `D:/wt/miri-gate` | `fix/miri-protector-arming` @ `02325b01` | merged fast-forward into KE16 (session record 2026-09-10) | contained (verified); clean | prune |
| `D:/wt/gates` | `fix/inherited-red-gates` @ `a0347509` (pushed) | contents not read | contained (verified); clean | prune |
| `D:/wt/aether` | `feat/aether-v2` @ `b6c41237` | Aether v2 | contained (verified); clean | prune |
| `D:/wt/stagetime` | `timing/per-stage-pyramid` @ `d11962a9` | reserved for MQ-01 | contained, 0 ahead | **move onto the trunk before timing** (a fast-forward; RK-4) |
| `D:/wt/tls-prefix`, `D:/wt/tls-fixb` | detached @ `8d13115e` | TLS probes; `tls-fixb` has uncommitted edits to `boyko_threadpool/src/{scope,worker}.rs` | contained | **review, then prune.** MQ-13 builds its arms from D-M6's merge commit (03 §5), so no queued entry needs the probes. Before the prune, the orchestrator saves `tls-fixb`'s uncommitted diff (`git diff`) to the session record and reads it (RK-11) |
| `.claude/worktrees/festive-lamport-c4fa90` | detached @ `e65a5673` (branch `claude/festive-lamport-c4fa90` at the same commit) | agent scratch | contained; clean | prune |
| `.claude/worktrees/wf_82be33ad-d8e-1` | `worktree-wf_82be33ad-d8e-1` @ `e65a5673` | agent scratch | contained; **not clean:** `.claude/settings.local.json` modified, `crates/boyko_ecs/tests/jump_table_probe.rs` untracked | review the two entries, then prune |
| `.claude/worktrees/trusting-ramanujan-0f8927` | detached @ `867dd734` (branch `claude/trusting-ramanujan-0f8927`, pushed) | agent work | **3 / 13, NOT contained;** `git cherry` finds `6a9871bb` (event-lane width hazard + worker-panic propagation) and `867dd734` (`docs/OPEN-QUESTIONS.md`) not patch-equivalent in `d552be05` | step A0 (02 §2): review before A2 merges and before A8; prune after A0 |
| (new, at A5) | `fix/visibility-sync-generation` (A9), in a lane worktree cut from `d552be05` | H-06 fix | — | A5 batch |
| — | `ecs` (`76ed41ff`), `master` (`7e908c87`, 612 ahead of `origin/master`), `phase5-gpucolumn` (`85d992d6`), `serialize-perf` (`b93ee369`), `feat/shadow-denoise-and-ssao-blur` (`b0b8b025`), `feat/agent-workflow-optimization` (`4cb9f0da`), `fix/claude-md-stale-doc` (`76ed41ff`), `worktree-agent-acaa90b48568bace1` (`2da64484`) | historic refs | — | not integration targets |

## 2. Merge order

Each step is a merge into `D:/wt/joltab` (the future trunk) unless noted. The architect could not
run git; the ancestry facts this order rests on were checked by the writer (§1).

0. **A0:** review `6a9871bb` and `867dd734` from `claude/trusting-ramanujan-0f8927` (02 §2). The
   panic half of `6a9871bb` is settled before step 2 merges A2.
1. **A1 commit + push `merge/ke16-into-ecsnative`.** The push happens only after UG-01 is green,
   including `production_reachability_census` (F1).
1a. **AH, the gate host moves to msvc (Q-7).** After step 1 and before step 2:
   - Merge `fix/census-post-lto-object` (`27ac8904`, one commit on `02325b01`), then `chore/msvc-host` (`a36ceaa4`, three commits on `e6115223`), into `D:/wt/joltab`. Both bases are merge bases with `d552be05` (§1), so each merge carries only the lane's own commits.
   - Then the Miri re-spell, the `TEMP` redirect and the re-bless (02 AH).
   - **No other lane merges `chore/msvc-host`.** Every Phase-A lane merged after AH is re-gated on msvc on the merged tree.
   - Main receives the recipes through A8, where `chore/msvc-host`'s `boyko_rhi_vulkan/src/device.rs` meets the owner's committed edit of the same file and is resolved by hand.
1b. **A1b** in `D:/wt/joltab`, after 1a.
2. **A2:** merge `fix/pool-panic-and-reservoir-loom`.
3. **A3:** `git log --oneline d552be05..fix/ke13-ke14`, re-test, merge.
4. **A4a:** the hot-path exception at `entity_reservoir.rs:405`, after A2, whose loom model edits
   the same file.
5. **A5, in this order** (smallest conflict first):
   - `chore/doc-gates`, `fix/asset-validate-prereqs`,
     `feat/golden-edsl-p0`, `fix/ddgi-host-hook` (its `runner.rs` change lands after A2).
   - `merge/ke16-into-render`'s one test, and the census test from `docs/ab-register-sync`.
   - `fix/visibility-sync-generation` (A9).
6. **A6:** `feat/reflection`. Merge base `5ec1699f` ([G]:414; re-read). Then run P38.4's anchor
   obligations.
7. **A7:** `feat/ui-advanced`. Merge base `5ec1699f`. Then the X-11 stress test.
8. **O2** (O1 is done, Q-5): the KE16 merge becomes a true merge with main, then main
   fast-forwards to it (§3).
9. **A4b:** the `QueryTypeId` exhaustion race (O1 is done). Read the owner's committed
   `query_type_registry.rs` first.
10. **A8:** `integ/unified` = the `[J]` result, merged with `feat/multi-paradigm-render` after O2. **This is a code merge.**
    - Main carries the owner's committed code (Q-5), and `boyko_rhi_vulkan/src/device.rs` is edited on both sides (step 1a).
    - Those files overlap A3's `query.rs` and A4b's `query_type_registry.rs`, and they touch
      `boyko_app`, `boyko_render`, `boyko_rhi` and `boyko_rhi_vulkan` — and also `boyko_physics`
      (`lib.rs`, `plugin.rs`), `boyko_log` (`codes.rs`) and `iters/query/{par_chunk,query_view}.rs`
      (00 §9 V-26).
    - Code conflicts are resolved by hand, file by file, never by UNION. UNION applies to diary
      docs only, followed by the census (session record: union merges re-import pre-ruling text).
    - Gates on the merged tree: UG-01 full, UG-10, UG-11, UG-12, UG-17, UG-18.
11. **After A8,** every rung and every document step merges into `integ/unified` in
    `D:/wt/joltab`; documents follow 02 §4.5. At each phase boundary (B, D-exit, E-exit), when the
    owner chooses:
    - first, any owner commit made on `feat/multi-paradigm-render` since A8 is merged into the
      trunk, with UG-10, UG-02 and UG-11 run on the merged tree;
    - then `feat/multi-paradigm-render` fast-forwards to the trunk.

    Nothing else commits on main after A8, so `--ff-only` succeeds.

**Checks before step 1.** `git merge-base --is-ancestor <tip> d552be05` for every lane marked
"contained" or "prune": done on 2026-09-17 (§1). Two lanes the plan had listed for pruning are
**not** contained (`feat/ecs-native-storage`, `claude/trusting-ramanujan-0f8927`); re-run the check
before any prune, since lanes keep moving.

## 3. The owner's steps

- **O1. Done (Q-5, 2026-09-17).** Committed in `5e86fe2d`, `f37650a6`, `f20bdafe` and `69cf3f79`, followed by `5aef7b39`, `b0bff31f` and `b716a5dc`. By rule, `.claude/settings.local.json` stays uncommitted, and so do the two model archives under `assets/models/` (one has a personal-use-only licence), which `b716a5dc` git-ignores. The seven overlaps recorded on 2026-09-10 are now committed code: `boyko_app/src/plugins.rs`, `…/query/{par_chunk,query}.rs`, `boyko_physics/src/{lib,plugin}.rs`, `docs/{FEATURE_MAP,SYSTEMS}.md`.
- **O2.** `git merge --ff-only merge/ke16-into-render` **will fail** (verified).
  - That branch was a direct descendant of `feat/multi-paradigm-render` at `0c18acfc` (session
    record; `0c18acfc` is an ancestor of `merge/ke16-into-render` and is the merge base of the two).
  - Main has since advanced by six commits: `dc35fae8`, `7770aa9e`, `ddaeed8e`, `d1e77f4f`,
    `f2691132` and `8a78ef6d`.
  - `git merge-base --is-ancestor f2691132 merge/ke16-into-render` is false, and so is the same
    check for `8a78ef6d`. Do a true merge, or re-merge main into `merge/ke16-into-render` first.
  - Main's side past `0c18acfc` now carries code (O1), so conflicts can arise in code as well as
    in `docs/`: the seven overlaps above.
  - **Procedure.** In `D:/wt/merge`, merge `feat/multi-paradigm-render` (`b716a5dc`) into
    `merge/ke16-into-render`. Resolve code by hand; resolve diary docs by UNION followed by the
    census. Run UG-01 full (`--workspace --no-fail-fast`, on msvc), UG-10 and UG-11. Then main
    fast-forwards to the result, which now contains main. The final resolution of the owner's
    seven files is the owner's step (session record).
- **O3.** Memory test of the workstation (RK-3).
- **O4.** Before F4's RF-V, RF-R and RF-T, the owner declares the render/rhi files quiet (critic pass 5, W3).
- **Goldens and GPU-windowed gates** for the ddgi and golden lanes.

## 4. Base branch and trunk policy

- **Base = `integ/unified`, cut at A8.** Reasons:
  - it is the only tree that is a superset of every lane's code;
  - the ledger's three citation trees collapse into one (B1);
  - every gate and every MQ receipt then names one tree.
- **One rung = one branch** `u/<rung-id>`, cut from the trunk. It merges back only when green on
  the post-merge tree, re-tested (`--no-fail-fast`).
- **Before cutting any rung,** no open rung may hold a file in its touch set (02 §4.3), and its
  fixed-order predecessors (02 §4.4) must have merged.
- **The trunk worktree** is `D:/wt/joltab`. No rung is developed in it. Phase B and every later rung
  run in the code pool, which opens at A8 (02 §4.1). Anchor re-derivations happen in the trunk
  worktree, as part of each merge (02 §4.2).
- **Documents.** From A8 on, the trunk is the only tree whose documents plan steps edit (02 §4.5).

## 5. Prune list

After the ancestry checks (re-run them first; §2):
- `D:/wt/threadpool`, `D:/wt/miri-gate`, `D:/wt/aether`, `D:/wt/gates` (all contained and clean
  on 2026-09-17);
- `fix/rejected-gpu-upload-leak`, only after `merge/ke16-into-ecsnative` is pushed past `d552be05`
  (until then its remote ref is the only pushed copy of its two commits; 00 §9 V-28);
- `.claude/worktrees/festive-lamport-c4fa90` (contained, clean);
- `.claude/worktrees/wf_82be33ad-d8e-1` after its two uncommitted entries are reviewed;
- the `tls-*` probes, after `tls-fixb`'s uncommitted diff has been saved and reviewed;
- `D:/wt/ecsnative` only after B4 has taken `tests/physics_vec_side_store_census.rs` from
  `ad0ebea4` and its two untracked census copies have been reviewed;
- `.claude/worktrees/trusting-ramanujan-0f8927` only after step A0 has merged its two
  non-equivalent commits or ruled them obsolete;
- not pruned: `D:/wt/refactor` (parked for F4); any `D:/wt/_targets/refactor` is deleted now (RK-11);
- after AH's pin commit: `D:/wt/_targets/joltab-gnu`, and the census's temp target dirs;
- after AH: `D:/wt/census` and `D:/wt/msvc`, once their branches are contained in the trunk
  (re-run `merge-base --is-ancestor` first).

Their target dirs under `D:/wt/_targets` are removed with them. Pruning is a git write, so it is
the orchestrator's or the owner's to run.
