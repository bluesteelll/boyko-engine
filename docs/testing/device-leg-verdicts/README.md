# Device-leg verdicts (`boyko_leg`) — design record

A device test that skips today prints `ok`: on a GPU-less box 108 of the 141 device-needing ignored
tests report a pass having measured nothing (`05-DEVICE-LEG-SWEEP-FINDING.md`, the first sweep of the
device leg, 2026-09-18). This folder holds the design that gives every such test its own verdict
(PASS / SKIPPED(class) / FAIL) and a runner that judges it against a committed per-machine baseline.

Status: **design closed by ruling, not implemented.** It lands as Phase B rung B5 on the trunk after
A8 and absorbs B3's ignore-prefix migration.

| File | What it is |
|---|---|
| `00-RULINGS.md` | The orchestrator's closing rulings — read this first; it is the erratum to rev 3 |
| `01-DESIGN-REV3.md` | The design, revision 3 (complete) |
| `02-REVIEW-OF-REV3.md` | The architecture critic's review of rev 3 (0 Blocking, 6 Important) |
| `03-TREE-INVENTORY.md` | Every skip route, worker and spawn site in the tree, as tables |
| `04-PRACTICE-SURVEY.md` | How libtest, nextest, Go, pytest, JUnit, GoogleTest, wgpu, Dawn, deqp-runner and the Vulkan CTS report a runtime skip, with sources |
| `05-DEVICE-LEG-SWEEP-FINDING.md` | The device-leg sweep report that found the defect |

File:line citations point at `D:/wt/joltab` (the integration line at `a1849541`) and `D:/wt/vkval`
(the boot-validation lane before it merged) as of 2026-09-18; re-derive them at the rung's cut.
