# boyko_leg design — CLOSED by orchestrator ruling (2026-09-18)

Authoritative text = `01-DESIGN-REV3.md` + this file (an erratum the implementing architect folds in as rev 3.1
before B5-0 starts). Review history: rev 1 (critic input truncated by the orchestrator → B1), rev 2
(0 Blocking / 7 Important), rev 3 (0 Blocking / 6 Important, all "gaps in the plan text, none needs a
redesign"). Closed by ruling rather than a fourth loop: every remaining item has an unambiguous
"needed" and none changes the architecture (rule: close when only Important remain).

Placement: Phase B rung **B5** on the trunk after A8, absorbing B3's ignore-prefix migration (01-DESIGN-REV3.md §8).
Priority: after B1–B4, before C1 (gates before code); it holds one code-pool worktree.

## Rulings on 02-REVIEW-OF-REV3.md

- **I1 (terminal device error mid-run reads PASS):** ADOPT. A once-only `HostLoopExit` witness
  (Normal / FenceWaitFailed / RenderErr, with the VkResult) written like `HostFramesPresented`;
  `judge_run` FAILs on any terminal device exit; G9 gains the fabricated-World row.
- **I2 (known rows keyed on panic text):** ADOPT. The validation callback records the set of
  `pMessageIdName`s into the per-context state (bounded, fixed capacity, overflow counted — the
  callback is a debug path, not a hot path, and still allocates nothing); validation `known` rows key
  on that ID set, so a NEW ID is FAIL. Substrings that are empty or match the UNMARKED / state-machine /
  oracle templates are rejected with exit 2.
- **I3 (default-leg sites switched on):** RULING — a default-leg (non-ignored) site is switched to
  validation-on at the cut ONLY if it is measured clean there; otherwise it stays off with a
  `leg-validation: off -- <finding id>` declaration in the pinned table (I5). Default-leg tests have no
  `known`-row channel by design, so a later red at a switched-on site is a real regression and UG-01
  goes red — correct. UG-01's posture: the gate shell carries NO `BOYKO_*` validation variable; each
  test's own `InstanceConfig` decides. State this in 03 §2 / tester.md at B5-4.
- **I4 (C0/C4 not green alone; lock sets):** ADOPT. `tests/engine_packages_census.rs`,
  `tests/production_reachability_census.rs`, `scripts/check_hotpath_exceptions.py` +
  `docs/HOT-PATH-EXCEPTIONS.md` (the moved `BOOT_LOCK` Mutex) join C0/C4 and the lock sets, and become
  §4.3 shared-file rows (B3, C1 and RP-2 edit the first two as well).
- **I5 (a red member leaves the leg without a finding id):** ADOPT all three: a Leg body may carry only
  `gpu*` or `generator` unless listed in a shrinking table with a finding id; `leg-validation: off`
  declarations live in a pinned table changed only in owner or document steps; `leg list` reports
  excluded Leg bodies.
- **I6 (window between B5-4 and C12):** ADOPT the per-package switch-on: rules (h1), (d), (i) turn on
  per package at the B5-4 and B5-5 merges with per-package floors.
- **Optional:** O1, O3, O4 (require a literal worker name or a const the scan resolves), O5, O6, O7, O8,
  O10 adopted. O2: a `flake` row kind (finding id required; stale if it never fires in the last N=10
  owner runs). O9: priority as stated above.
- **Open questions:** (1) UG-01 posture — see I3. (2) the oracle records `pMessageIdName` — see I2.

## Owner question still open
F1: is `assets/vg_corpus` fetched on the owner's box (yes/no)? If no, the corpus members get
`expect … payload-absent` rows (O7 lists the default-leg payload members too).
