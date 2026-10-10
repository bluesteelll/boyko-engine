# Engine runtime as ECS - decisions on the open questions

- **Date:** 2026-09-11
- **Design:** [ENGINE-RUNTIME-ECS-DESIGN.md](ENGINE-RUNTIME-ECS-DESIGN.md), rev 2, section 14 ("Owner questions vs architecture decisions")
- **Research:** [ENGINE-RUNTIME-ECS-RESEARCH.md](ENGINE-RUNTIME-ECS-RESEARCH.md)
- **Sibling:** [../physics/PHYSICS-ECS-UNIFICATION-DECISIONS.md](../physics/PHYSICS-ECS-UNIFICATION-DECISIONS.md), taken under the same delegation

## The delegation

The owner, 2026-09-11, verbatim in translation: *"Decide all the questions yourself, whichever is best
for performance."*

The criterion is the throughput of the frame. Where two options cost the same on every hot path,
the tie-break is the owner's standing goal: "bring everything as close as possible to the ECS
paradigm and make one unified system". A decision taken on reasoning names the measurement or gate
that would overturn it. Q5, permission to time on the owner's workstation, is not a design question
and is governed by the owner's standing rule.

## Decisions

| # | Question | Decision | Recommended in the design? | Overturned by |
|---|---|---|---|---|
| Q1 | Assets as entities | **(a) Assets are entities** | yes | A per-draw handle-resolution microbench regressing beyond its band |
| Q2 | `EnginePlugins` composes the UI by default | **(b) Opt-in `UiPlugins`** | no - the design recommended (a) | none expected |
| Q3 | Multiplicity in v1 | **(b) Window and player are entities now** | no - the design recommended (a) | none expected |
| Q4 | Delete dead paths (`Gpu3dInstance` + `Render3dPlugin`, `TelemetryStream`, legacy UI scratch) | **(a) Delete** | yes | none |
| Q5 | Timing permission | **Only on the owner's word that the machine is quiet** | - | not a design question |

### Q1 - assets are entities

- **Hot paths are unaffected.** The draw path does not resolve an asset handle per draw. Instances
  carry pre-resolved GPU indices (bindless slots and mesh ranges), so an entity-backed handle is not
  on the per-draw path.
- **Where the gain is.** One store per asset kind replaces the duplicated tables the design counts:
  19 copies of 7 data become 7. Refcounting becomes a count-only relation (EK15b), and asset change
  detection becomes ordinary `Changed<T>`. That removes a sync step and its memory rather than adding
  one.
- **Where the cost is.** Structural changes happen at load and unload time, not per frame.
- **Gate.** A microbench of handle-to-index resolution, at the rung that switches `Handle<T>` minting,
  must not regress beyond its band.

### Q2 - the UI is opt-in (`UiPlugins`), not composed by default

- **What the default costs.** A system on the schedule costs dispatch even over an empty query. The
  design itself sets a budget for render-schedule dispatch (at most 2 µs per frame at ~20 systems),
  which shows the cost is not zero.
- **Who pays.** An application with no UI should not pay for the UI's systems every frame. Opt-in
  means only the apps that build a UI pay for it.
- **What does not change.** The UI stays a first-class part of the one engine: same storage, same
  scheduler, same kernel features. Opting in is only a question of which plugins an application
  composes; it adds no second path.
- **What remains to settle.** The shipped apps that draw a HUD (`boyko_demo`, the playground) add
  `UiPlugins` explicitly.

### Q3 - window and player are entities now

- **Cost.** Both options cost the same per frame. Per-window and per-player data are touched once per
  frame per instance, so a component on an entity costs nothing measurable over a singleton resource.
- **Tie-break.** The owner's unification goal decides, and it cuts one way.
  - The ledger's gap 8 was decided the same way: window data goes as components on a window entity
    (see `docs/memory/RUNTIME-DATA-LEDGER.md`, rev 2).
  - Choosing singletons now would build a v1 that the multi-window and multi-player model must later
    migrate away from.
- **Consequence for the design.** The rungs that assume one `WindowSurface` resource and one
  `ActionState<A>` per `A` are re-cut in the next revision. `ActionState<A>` becomes a component on a
  player entity, and window data becomes components on a window entity. The OS-owned handles stay
  out of scope.

### Q4 - delete the dead paths

- **Cost of keeping them.** Dead code costs compile time, binary and I-cache footprint, and G-FORM
  rows that could never shrink.
- **Why delete.** Nothing live reaches these paths, per the design's own evidence, so deleting them
  is free.

## What this file does not do

- **It does not edit the design.** Rev 2 stays as written. The next critique pass and its patch are
  told these decisions: Q2 and Q3 differ from the design's recommendations, and the patch re-cuts the
  affected rungs.
- **It does not reconcile the two designs.** The span primitive (engine EK14 / ED15 against physics
  K7), the deferred release (K6 / K6′) and the rung interleaving are reconciled in the unified plan,
  which is built from both designs, the allocator design and the runtime-data ledger.
