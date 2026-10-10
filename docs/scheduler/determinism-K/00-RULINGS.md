# Schedule determinism (lane K) - orchestrator rulings (2026-09-19)

The lane exists because an ordering edge that carried no data moved six hwrt TAA goldens
(`00-DIAGNOSIS.md`): `visibility_sync` enables `RenderEnabled` through a deferred command, `gather_mesh_draws`
filters on it, the two had no ordering path, and in the pinned order frame 0 drew no meshes. The same diagnosis
measured that deferred commands inside one apply window run in completion order (34 orders in 34 frames at W > 1),
which breaks the owner's replay rule (identical at any W and on any machine). The instance of the render bug is
fixed in the light-table lane (R4, "every render reader after its writer"); this lane fixes the class.

Files: `00-DIAGNOSIS.md` (the bisection), `01-RESEARCH.md` (Bevy, flecs, Unity DOTS, EnTT and our executor),
`02-DESIGN-REV1.md` + `03-REVIEW-OF-REV1.md` (REVISE: 1 blocking, 5 important), `04-DESIGN-REV2.md` +
`05-REVIEW-OF-REV2.md` (REVISE: 0 blocking, 3 important).

## Rulings on rev 1 (folded into rev 2)

- C1: every read route of the enable column declares a read — `Enabled<T>`, `Disabled<T>`, `IsEnabled<T>` and
  the runtime enable terms (a query that can carry them reads every enable tag unless it declares its tags).
- W1: open-ness is accounted per parameter; an undeclared `GpuCompute` system is `Unknown`.
- W2: the determinism obligation is stated over the whole frame, with a split-retire case in the W-sweep.
- W3: the dispatcher lane has a growth-invariant identity (lane 0, flattened last).
- W4: a standing leg runs the ratchet with `--features hwrt`.
- W5: the ratchet pins can only lose lines.
- K1a/K1b land ahead of the unified plan's B1–B3: approved, because determinism is a bug (bugs before features).

## Rev 2: CLOSED by ruling

No blocking remark remains. The implementing lane folds these in as rev 2.1 before its first commit.

- **W-A — `send_event` keeps `&self`.** Rev 2 made every non-writer send take `&mut self` to close a real race
  (two unattached threads both writing lane 0 through `&EcsMaster`). That overturns two recorded decisions
  without citing them: Aether E5 (`docs/aether-v2/DECISIONS.md:539-580`, a CAS-claimed host lane with one
  claimer enforced, chosen after measuring the same race) and U-21 / D-E20 (`&self`, a worker gets
  `Err(EventSendOffDispatcher)`). The claimed lane closes the race just as well and keeps the documented escape
  hatch for main-thread and FFI senders. Ruling: lane 0 stays the dispatcher/host lane, fixed and flattened last;
  **every** lane-0 send (the dispatcher's apply path included) goes through the single-claimer CAS, and a failed
  claim returns an error, never a silent drop; a worker still gets `Err(EventSendOffDispatcher)`. Host sends are
  timing-dependent by nature and are replay INPUTS (recorded), outside K1's claim. K1b's file list gains the
  documents that describe lanes (`02-ORDER-OF-WORK.md:323`, `01-KERNEL-CONTRACT.md:613`,
  `aether-v2/{DECISIONS,EVENTS}.md`, the `constants.rs:396-432` doc and its const-assert).
- **W-B — the ratchet binary layout:** one `EnginePlugins` build per test binary (component hooks are
  process-global; Main and Fixed come from one `finish()`); software rows `cfg(not(feature = "hwrt"))`; the canary
  on a bare `ScheduleBuilder`; the expected `running N tests` stated per leg; the CI step checks that the hwrt row's
  test NAME passed, not `N >= 1`.
- **W-C — the ratchet cannot be laundered:** one digest over FROZEN, RESOLVED, ALIASES and ALLOWED together; the
  alias map is injective over the whole FROZEN name set (`new` is not a name already in FROZEN).
- **O-1:** D8b's reserved-not-spawned check keys on a watermark taken at the start of the apply phase (the
  immediate arm as written can never fire). **O-2:** lanes are never released on a schedule rebuild — named in
  the limits. **O-3:** the runtime-term census counts calls, not lines.
- **Review OQ1:** the split-retire reference model takes S's duration from the configured busy-wait.
  **OQ2:** the implementing lane states each leg's test count after the W-B layout.

## Lane order

K is a bug-fix lane and runs as soon as a build directory is free: it is cut from the line after the light-table
lane merges (so the ratchet's frozen set is taken with R4's render edges in), and it reuses that lane's target
directory. The physics lanes continue in parallel on their own target.
