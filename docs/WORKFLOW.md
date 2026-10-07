# Workflow rationale — the orchestrator's reference

This file holds the orchestrator-facing rationale moved out of [CLAUDE.md](../CLAUDE.md) on
2026-10-07 to keep the always-loaded file small; CLAUDE.md holds the rules. Every section below is
the original CLAUDE.md text, moved verbatim (only three relative link targets were re-based from the
repository root to `docs/`). Where a passage restates a rule, the shortened rule in CLAUDE.md is
authoritative. Numbers are as measured on the dates they carry and are not re-derived here. A
cross-reference that leaves its section ("the four commands above", "rather than only here") points
into CLAUDE.md as it stood before the move.

## Build-command flags

⚠️ **`--workspace` and `--no-fail-fast` are both load-bearing, and each was added after a
measurement, not for tidiness.**

- **`--workspace`**: without it the virtual manifest at the repo root type-checks a subset and
  reports success — the 2026-07-23 audit found the whole CI vacuum-green this way.
- **`--no-fail-fast`**: `cargo test` **stops at the first failing target**, so one known-red target
  *shadows* every target ordered behind it. MEASURED 2026-08-10: the workspace had **three** red
  targets while every report said "green except the known `internal_docs_anchors`" — the trybuild
  fixture `token_use_after_submit_rejected` had been red for 87 commits (a line added, its `.stderr`
  never re-blessed) and `cluster_bound_arraylength`'s shader set had been stale since VG R3's batch
  cull gained a bound query. Neither was visible until the flag was passed. **"Green" without this
  flag means "green up to the first thing already known to be red".**

## Ignored suite

The four commands above run **none** of the 164 `#[ignore]`d tests (143 unconditional + 21
`#[cfg_attr(miri, ignore = …)]`). Every one of them now states its requirement at the site, and
[tests/ignore_reasons_census.rs](../tests/ignore_reasons_census.rs) fails the build if a new one does
not — a bare `#[ignore]` is the **third** way to make a check disappear, after `unsafe` and
`#[allow(clippy::disallowed_types)]`, and it now carries a written rationale like the other two.

**Leg: device-free ignored tests.** Runs on any machine, no GPU, ~0.03 s. **Covers 1 test.**

```powershell
cargo test -p boyko-log --test l14_sink_policy -- --ignored --test-threads=1
```

The output must read `running 1 test`. A `running 0 tests` line is a vacuous pass, not a pass —
see the `#![cfg(miri)]` trap below, which produced exactly that.

**Leg: device-needing ignored tests.** A machine with the GPU; the orchestrator or the owner runs
it, per-binary with `--test-threads=1`. **Covers 135 tests** across `boyko_app`, `boyko_render` and
`boyko_rhi_vulkan`. Four of them need a non-default cargo feature and are not even *compiled*
otherwise (`--features hwrt` ×3, `--features spec_constant_smoke` ×1); ~131 additionally sit behind
`#![cfg(windows)]` and vanish on Linux. There is no single command — each binary has its own
env-var protocol in its module header (`BOYKO_DISABLE_VALIDATION`, `BOYKO_HZB_DUMP`,
`BOYKO_WINDOW_FRAMES`, …).

**Leg: Miri.** `cargo +nightly miri test` already carries **22** of the ignores — the 21
`cfg_attr(miri, …)` sites (which run *natively* and are skipped only under Miri) plus
`miri_fixed_loop`'s one plain ignore. None of these belong to either leg above.

**Six ignored tests belong to no leg at all**, and must not be swept into one: three *generators*
that assert nothing and emit source to paste (`dump_maximal_frame_barrier_stream`,
`dump_vb_unsplit_barrier_streams`, `dump_vb_split_barrier_streams` — the second's own doc warns
that running it casually re-measures the baselines the split is compared against), two *deferred
milestones* that are RED by design until M2's JCGT cubic lands
(`brick_field_is_conservative_lower_bound`, `trilinear_reconstruct_is_a_tight_lower_bound_in_r1`),
and one *timing probe* documented "NOT a CI gate" (`no_starvation_every_worker_makes_progress`).

⚠️ **The device-free leg is one test, and it is an explicit invocation rather than a filter,
because the partition CANNOT be derived from the reason strings.** A keyword classifier over
`{GPU, RTX, Vulkan, windowed, device, dispatch}` puts 10 of the 143 on the device-free side, and
**8 of those 10 are wrong** — and wrong in the direction that produces a green:

- `negative_chained_barrier_hazard` and `a5_gpu_off_vs_on_wall_clock_ab` **do** need a device; their
  reasons name the *hazard* and the *purpose*, not the requirement. Both call `boot_*_or_skip`, so
  on a GPU-less box they return early and **pass** — a leg that includes them reports green while
  measuring nothing.
- `miri_app_driver_substeps_and_hold` is inside a `#![cfg(miri)]` file, so natively it does not
  exist. Filtering for it prints `running 0 tests` and exits 0. **Measured while writing this leg.**
- The other five are the generators / deferred / flaky above, which would pass meaninglessly, fail
  outright, or flake.

The property the leg needs — *what does this test require, and is it a gate at all* — is simply not
what a prose reason answers. **Making it mechanical takes a reason PREFIX from a closed vocabulary**,
checked by the same census: `#[ignore = "<class>: <prose>"]` with `class` one of `gpu`,
`gpu-windowed`, `gpu-cap` (RT / ray-query / `VK_KHR_pipeline_executable_properties`), `feature`,
`solo` (device-free, needs `--test-threads=1`), `slow` (device-free, wall-clock budget),
`miri-slow`, `generator`, `deferred`, `flaky`. Today's tree maps onto it exactly — 135 `gpu*`/
`feature`, 1 `solo`, 1 `miri-slow`, 3 `generator`, 2 `deferred`, 1 `flaky` — so the migration is
mechanical, and afterwards each leg is a `grep` and every new ignore picks its own leg at the site.

## Model routing

**Model routing** (set via each agent's `model:` frontmatter): **every role runs on Opus** — owner decision, 2026-07-26, and as of 2026-09-07 there is finally a MEASURED diff behind it rather than only a judgement.

**One exception (owner, 2026-09-30, after a re-run of the measurement below on Sonnet 5.5: 26/27, 1 confident-wrong):**
the `tester` runs on Sonnet 5.5 through the lane scripts' per-call `model`; `code-reviewer`, `results-analyst` and
every verifier stay Opus. Watch the escape rate of Sonnet-tester GREENs.

**The measurement (2026-09-07).** 27 retrieval questions over this tree, each with a ripgrep-verified
ground-truth passage and phrased WITHOUT the target's own distinctive vocabulary, run through an
identical brief on three tiers. The task is the most *mechanical* thing an agent here ever does —
search, read, report a file and line span — so it is the friendliest possible case for a cheaper tier:

| tier | found | rank-1 | TIGHT MRR | **confidently WRONG** | returned nothing |
|---|---|---|---|---|---|
| haiku | 14/27 | 14 | 0.519 | **8** | 3 |
| sonnet | 23/27 | 22 | 0.833 | **4** | 0 |
| opus | **27/27** | **25** | **0.957** | **0** | 0 |

The decisive column is the fourth, not the first. Sonnet asserted "I found it" on **four** questions
where its answer was wrong; Opus did so **zero** times. A confident wrong answer is this
repository's own catalogued failure mode, and it is the one a downstream reader cannot detect.

⚠️ **A six-question pilot of the same experiment said Sonnet TIED with Opus (6/6 each) and was
wrong** — those six happened to be questions Sonnet gets right. n=6 could not separate the tiers;
n=27 separated them by four answers and by all of the false confidence. Any future routing
measurement needs at least ~25 items, and must score *confident-but-wrong* separately from *missed*.

**The second measurement — the development workflow itself, from this repository's own history.**
The roles named below ran on Sonnet until 2026-07-26 and on Opus after, one repo, one orchestrator:
a natural before/after. Matched 43-day windows either side of the switch (545 vs 470 commits):

| signal | Sonnet era | Opus era | ratio |
|---|---|---|---|
| `fix:` commits — actual defect repairs | 10.6% | 10.6% | **1.00×** |
| commit-subject length — the style confounder | 10.4 words | 14.7 words | 1.41× |
| **numeric corrections** per 100 commits ("19 stale sites, not the 9 reported") | 4.0 | 13.4 | **3.32×** |
| **self-refutations** per 100 commits (the workflow catching its OWN output) | 7.9 | **50.9** | **6.45×** |

Commit style did become more discursive, which is why the raw "admits something was wrong" rate
proves nothing on its own — but self-refutation grew **4.6× faster than subject length**, so prose
cannot explain it. And the repair rate is *identical*: the workflow does not ship more fixes, it
finds far more falsehood in what it and its predecessors had already written — "the gate that passed
its own red mutation", "the gate caught its own author twice", "two of Rev 10's own repairs were
wrong, and the review caught both", "3 of 5 repairs first made it worse". The matched Sonnet window
contains **two** self-refutations, one of them a plain race fix.

⚠️ Confounded, and the confounder is named: the Opus era coincides with campaigns *about* gate
quality (diagnostics ladders, the anchors gate, VG critique rounds), which generate self-refutation
by their nature. Topic selection is part of the effect; the 6.45× is not attributable to the tier
alone. What survives the confounder is the pairing with the retrieval table above — **two
independent measurements, one synthetic and one historical, both isolating the same faculty: not
capability, but self-scepticism.** Opus's edge is knowing when its own output is wrong, which is
precisely the axis this repository's entire failure taxonomy runs along ("a check that could not
fail", "green from emptiness", "a confident wrong answer").

**So the downgrade question is answered in the negative, with numbers, for the one role that can be
measured cheaply — and the roles above it are protected LESS, not more.** Only facts are gated
downstream, never judgement: `cargo` catches code that does not compile but not UB in an `unsafe`
block; a `tester`'s pass/fail is gated while a test that *cannot fail* reads as green; a
`doc-writer`'s output is gated by nothing at all (repairing doc rot introduced new falsehoods in 3
of 5 measured attempts). Do not downgrade any role on the argument that it is "mechanical" — that
premise was tested here and failed. **Optimise effort, batch size and redundant arms instead of the
tier** (`effort:` is a per-call knob on Opus and keeps its judgement).

The previous split put `developer`, `tester`, `researcher`, `doc-writer` and `project-analyst` on Sonnet as "mechanical / gathering" roles. That premise did not survive contact with this codebase: the roles it called mechanical are the ones that catch the campaign's defects. On the VB-SV0 stage alone, implementers refuted the orchestrator's own prescriptions **nine times** — a ULP tolerance that was wrong in form because the leaf ends in a cancellation, a `-D`-push route replaced by an `OpArrayLength` bound the orchestrator had not considered, a z-range check whose prescribed site would have panicked at boot in every process, and a NaN claim corrected on the wrong leaf. None of that is transcription work. Do not downgrade any role without a measured before/after quality diff — this applies to all nine now, not only to `code-reviewer` (which remains the last line against `unsafe`/atomics UB). The orchestrator itself stays on the session model.

## Token economy

Measured 2026-10-07: 1,148 agents, 66.5k calls; data `D:/tmp/phys-orch/token-audit/`.

Cost ≈ turns × context: ~95 % of volume is cache re-reads, and 79 % of what is re-read is context growing inside ONE
agent. The NUMBER of agents is not the driver (spawn + brief + re-orientation ≈ 5 %), and every modelled stage merge
cost MORE (+0.4 … +8.6 %), because the merged agent re-reads the earlier stage's context on every later turn.

- **Do not merge stages to save agents.** Merging also kills independent refutation (tester ∥ reviewer, triage → fix).
  Parallel arms buy wall time, not tokens: run them when their inputs or roles differ.
- **Parallel checkers start from one context pack, not N explorations.** Reviewers spend 42–48 % of their tokens
  before their first action, triage 61–65 %, cut 58 %. When ≥ 2 agents of a run need the same orientation, one scout
  writes a ≤ 15k-token map (files + line ranges, key facts, the delta) to disk and every brief points at it; the
  checkers stay separate agents. The shared prompt prefix (system, CLAUDE.md, brief) is already served from one cache.
- **Keep the cache warm.** Subagents use the 1-h cache TTL (`subagentPromptCacheTtl` in `.claude/settings.json`);
  23.5 % of cost was full re-writes after waits past the old 5-min TTL. Launch a long gate detached in its own console
  (`Start-Process … -WindowStyle Minimized`; never `run_in_background`/`&` — a reaped shell's children die 0xC0000142),
  have it write PID / DONE / exit codes / a per-row summary, and wait in bounded slices that check the PID is alive and
  the gate is within its time budget; past budget is a hang — stop it by its own PID.
- **Logs live on disk, not in context.** Tool outputs over 5k tokens, re-read for the rest of an agent's life, are
  27.9 % of all re-reads. Use `rg -n` + ranged reads (≤ ~200 lines); read summaries, verdict lines and failure excerpts,
  never whole logs. Check `test result:` targets BY NAME against the expected set (equal counts over different sets are
  a vacuous green).
- **Hand off through files.** Briefs carry paths + section names, never pasted documents; every report is saved;
  a resume = a commit + a ≤ 1-page `handoff.md`. Split a long agent only at a commit, past ~350k context.
- **One owner per heavy gate per round.** Developer: check/clippy, its red-firsts, the gates its diff touches.
  Tester: workspace tests, pins, Miri, anchors. (Both ran a workspace check in 21 of 38 lanes.)
- **Rounds ≥ 2 are delta-scoped**: the confirmed items' red-first on parent and HEAD, ≥ 2 own mutations aimed at the
  fix delta and at the gates the last round added (S5 r2's only defect came from one), the touched gates, workspace
  check/clippy. One full workspace run on the lane's final HEAD before merge.
- **Keep every checking arm** (critique, triage, tester mutations, review each round — together ≈ 9–10 % of cost):
  triage confirmed a defect in all 13 rounds; SR review r3 caught a liveness defect after a green tester. Cut an arm
  only on a measured escape-rate diff over ≥ ~25 items. Lower effort was measured and rejected.
- **Small always-loaded floor.** Each 1k tokens in CLAUDE.md / MEMORY.md costs ≈ 0.29 % of all spend: MEMORY.md
  ≤ ~15 KB, measurement tables live in docs; typed agents start at 40–48k vs `claude` 72–89k — prefer typed agents.
- **Orchestrator.** Workflows return verdicts + paths (≤ ~2k tokens), not full agent texts; read reports by section;
  restart from the memory checkpoint past ~400k context.

## Russian docs freeze

- ⚠️ **`docs/ru/` is FROZEN as of 2026-09-07 — owner decision — and its pairing rule is WITHDRAWN.** The English documents are the source of truth and are the only side maintained. Do not update the Russian side when the English one changes, and do not translate new documents into it.
  - The freeze was taken with its cost stated and accepted: the pair **will** diverge, which is exactly what the withdrawn rule warned about — a reader cannot tell which side is current and finds out only by acting on the stale one. That is why the freeze is announced in [`docs/ru/README.md`](ru/README.md) itself, where the reader lands, rather than only here.
  - It began from a synchronised state: both sides were last written by `efd7735f` (2026-09-03), verified by `git log -1` per path, so the divergence is measurable from that commit forward rather than unknown.
  - A future session must NOT "repair" the divergence by re-syncing or by deleting the directory. Either is an owner call, not a tidy-up.

## rust-analyzer

`rust-analyzer` serves every `.rs` file through the `LSP` tool, declared by a machine-local plugin at
`~/.claude/skills/rust-analyzer-local/.lsp.json`. Prefer it for anything about a symbol's identity or
its relations, because it answers out of the compiler front-end instead of an index that can be
stale:

- `goToDefinition` / `findReferences` / `goToImplementation` — exact and workspace-wide.
- `hover` — type, doc comment, and the **computed** layout (`size = 312 (0x138), align = 0x8, no
  Drop`), which is load-bearing in a codebase whose principles are about layout.
- `workspaceSymbol` — locate a type or function by name across every crate.
- `incomingCalls` / `outgoingCalls` — call hierarchy, for tracing a hot path.

Use `Grep` (it is ripgrep) when the question is literal or pattern-shaped — a `// SAFETY:` census,
an `#[allow(clippy::disallowed_types)]` sweep, a string inside a shader — and `semble` (below) when
the question is about meaning in prose rather than about one symbol.

⚠️ **The server covers ONLY the projects listed in `settings["rust-analyzer"].linkedProjects`, and
that key REPLACES project auto-discovery rather than extending it.** Two consequences, both measured
2026-09-07:

- A file in a worktree that is not listed gets **syntax only**: `documentSymbol` answers in full
  while `hover`, `findReferences` and `workspaceSymbol` come back **empty**. That empty answer is
  indistinguishable from "no such symbol", so it is a silent wrong answer, not an error.
- The main checkout must never be dropped from the list, or fixing a lane breaks what already
  worked.

The list therefore carries the main checkout plus whichever lanes are under work, and lanes are
removed as they finish: each project costs several GB of RSS, growing with query volume. When two
trees are linked, `workspaceSymbol` returns one hit per tree for identically-named packages —
tell them apart by path form, since a linked lane answers with an absolute path and the main
checkout with a relative one.

## semble

Rationale in this repo lives in Markdown and in long comment blocks, so the answering words often
are not the asking words. `semble` (machine-local, `pip install --user "semble[mcp]"`) indexes
`.md` and `.rs` — tree-sitter chunking carries comment nodes verbatim — and re-validates its cache
on **every** search, so it cannot go a month stale the way a manually-rebuilt index can.

Use the wrapper, [`tools/semsearch.py`](../tools/semsearch.py), rather than `semble` bare — it adds
the cross-encoder rerank stage that carries most of the measured value, and it prints both orders:

```
python tools/semsearch.py "<question>" <tree> -k 8
```

Bare `semble search` also works; its `--content` defaults to **code only**, so pass `all` (or
`docs`) there or prose is silently not searched.

**The configuration is a measured optimum over 27 ground-truthed questions, not a guess.** Every
number below is `hits@10` on the strict metric (the returned span must actually overlap the target):

| configuration | hits@10 | recall@40 |
|---|---|---|
| semble as shipped — chunk 750, no rerank | **8** | 11 |
| chunk 1500, fused rerank | 12 | 14 |
| **chunk 4500, pool 40, fused at α=0.5** ← the wrapper's defaults | **16** | **17** |

**hits@10 doubled and recall went 11 → 17**, from a chunk constant, a pool cap, an 80 MB
cross-encoder and a fusion weight — no new model, no torch, no npm. Four things were each measured
and each is counter-intuitive, so do not "simplify" them away:

- **The chunk budget is the biggest single lever, and the shipped default is the worst setting.**
  750 chars (~190 tokens) against rationale blocks averaging ~2,200 splits a block into ~4
  fragments, which semble's own ranker then penalises (2nd fragment from a file ×0.5, 3rd ×0.25).
  Swept: 750 is last of {750, 1500, 3000, 4500, 6000} on every metric. The wrapper patches the
  constant at call time rather than editing `site-packages`, because a patched install is silently
  reverted by the next upgrade.
- **A deeper pool is worse AND dearer.** Recall does not improve past 40 (5/6 at 40, 150 and 400),
  while reranking degrades monotonically and cost grows 10× (27 s → 272 s/query). This reproduces
  arXiv 2411.11767's finding that a pointwise reranker beats the retriever alone in barely half of
  measured cases at high K.
- **Fusion is required; both extremes lose.** At chunk 4500, hits@10 is 11 with the first stage
  alone, 14 with the cross-encoder alone, and **16 fused at α=0.5**. Replacing the order outright
  moved one target from rank 38 to 2 while pushing a rank-1 target down to 9 — which is why the
  wrapper prints `was=#N`, and why `--no-rerank` exists.
- **Do not change the embedding model.** The default `potion-code-16M-v2` measured best;
  `potion-retrieval-32M` and `potion-base-32M` both scored worse and dropped a code target out of
  the top 40 entirely, and merging all three candidate pools recovered no recall the default did not
  already have.

**The split is measured, not assumed (2026-09-07), and it goes both ways:**

- **Broad topical question → semble.** "Which subsystem decides shadow-map stability" against
  ripgrep is `shadow` = **6,265 occurrences in 270 files**, useless as a starting point; semble
  returned 8 ranked hits including the right plan document and the doc comment recording why the
  shadow-map camera once pointed 180° away.
- **Specific rationale with a guessable rare word → ripgrep.** "Why can the yield count not be
  tuned" was found by `tune|tuned|tunable` scoped to one crate — **3 files**, answer among them —
  while semble **missed it in the top 5 even in the tree that holds it**.
- Rule of thumb that follows: if a keyword guess would return few files, grep; if it would return
  hundreds, let semble rank.

⚠️ **A bigger embedding model is not the fix, and it was tested twice.** On a single question
`potion-retrieval-32M` (249 MB) looked better than the default and `potion-multilingual-128M`
(1002 MB) looked worse; on the six-question set the default won outright. The reason is structural:
every one of them is a *static* embedding — token vectors summed with no attention — so a paraphrase
bridge like "cannot be tuned" ⇒ "no constant threshold can certify it" is out of reach **by
construction**, and semble loads only `model2vec.StaticModel`. That is the ceiling a cross-encoder
steps over, because it reads the query and the chunk together.

The failure was diagnosed before it was fixed, and the diagnostic is reusable: a **near-verbatim**
query retrieves the target at rank 1 with score ~**0.0197** — measured three times, on three
different targets — while a paraphrase of the same question leaves it outside the top 5 at ~0.009.
So the chunk is indexed and recall is adequate; only the ordering fails. **Treat a first-stage top
score below ~0.012 as "nothing really matched"** and prefer a ripgrep guess on a rare word from the
question. Semble's own `rerank` is a lexical heuristic (file/identifier boost, path penalties over
an RRF fusion with BM25) and is already on — it is not a cross-encoder, so the two stack.

⚠️ `semble mcp` does not exist — it exits 2. The CLI satisfies the integration need on its own.
Shader grammars are absent (`wgsl`, `hlsl` warn and fall back to line chunking).
