# Physics ECS unification - decisions on the open questions

- **Date:** 2026-09-11
- **Design:** [PHYSICS-ECS-UNIFICATION-DESIGN.md](PHYSICS-ECS-UNIFICATION-DESIGN.md), rev 2, section 14 ("Owner questions vs orchestrator decisions")
- **Research:** [PHYSICS-ECS-UNIFICATION-RESEARCH.md](PHYSICS-ECS-UNIFICATION-RESEARCH.md)
- **Code citations** refer to the tree the design was taken on: `D:/wt/joltab`, branch
  `merge/ke16-into-ecsnative` @ `d11962a9`. `enable_store.rs` is
  `crates/boyko_ecs/src/ecs/core/component/enable/enable_store.rs` there. The TLS measurement
  document exists on that branch and on `merge/ke16-into-render`, not yet on this checkout's branch.

## The delegation

The owner, 2026-09-11, verbatim in translation: *"Decide all the questions yourself, whichever is best
for performance."*

So the five questions the design put to the owner are decided here, on one criterion: the throughput
of the physics step, at the Jolt-pyramid scale (1240 bodies) and at a large scale, on this kernel's
storage layout. A decision taken on reasoning rather than on a measurement must be falsifiable, so
every decision names the measurement or gate that would overturn it, and the rung at which that
measurement is taken.

One question is not a design question and is not delegated: permission to time on the owner's
workstation (Q4). The owner's standing rule governs it.

## Decisions on the owner questions

| # | Question | Decision | Overturned by |
|---|---|---|---|
| Q1 | How gameplay sees contacts | **(a) Kernel events only**: contact and sensor enter/exit | A consumer whose per-frame "currently touching" query, rebuilt from events, measures costlier than the structural churn a relation would add |
| Q2 | Soft-body particle storage | **(a) K7, the segmented dense column** | The S0 rung's soft-body bench (`soft_colored_sp4`) regressing beyond its band |
| Q3 | How gameplay sees sleep | **(a) `BodyGate` in the group, plus sleep/wake transition events** | None expected; gated by frozen-body byte-identity and the census |
| Q4 | Permission to run timings | **Only on the owner's word that the machine is quiet** | Not a design question |
| Q5 | Where pose and velocity live | **(a) `RigidBody` stays an authored table component; the solver works on the derived dense group; writeback touches only awake dynamic bodies** | SP-1 at rung R0: if (b) is faster beyond the band at W=8 on BOTH the pyramid and a 20k-body scene, revisit before U4 |

### Q1 - events only

- **The cost of an event.** An enter or exit event costs O(transitions). S7 writes it once, in a
  serial merge walk over manifolds that S3 has already built.
- **The cost of the alternatives.** A `Touching` relation or a `Contact` component turns every contact
  change into a structural change: a relation-table insert or remove, or an archetype move. That is
  paid on the command/apply path, per contact, per step. A settling pile makes thousands of those.
- **The consumer.** Gameplay that needs "is X touching Y right now" keeps that set in its own system,
  built from the event stream. It pays only for the pairs it cares about.

### Q2 - K7 segmented dense column

- **Contiguity.** The particles of one soft body sit contiguously in one bank, SoA per attribute. The
  per-body solve streams them, and the parallel particle loops chunk cleanly.
- **One code path.** The same kernel feature serves animation bone arrays and UI text runs. That is
  one path kept hot in the instruction cache instead of three.
- **Why not the alternative.** ADVANCED D-8's resource row bank is as contiguous, so it is as fast.
  But it is a second storage beside the ECS, which the owner's orders rule out for no speed gain.

### Q3 - `BodyGate` plus transition events

- **Where the solver reads sleep.** Every solver lane reads sleep in every pass. In `BodyGate` it is a
  bit in the co-slotted group the solver already streams, so it costs no extra load.
- **Why not an `EnableTag`.** An `EnableTag` lives at `(archetype, row)` (`enable_store.rs:9`). Reading
  it from the solver means either one random lookup per body per pass, or a second copy kept in sync.
  Toggling it also needs `&mut EcsMaster` (`enable_store.rs:23-24`), which is a structural operation
  in the middle of a step.
- **How gameplay sees it.** Sleep and wake reach gameplay as events, O(transitions).

### Q5 - authored `RigidBody` plus a derived solver group

1. **The solver's working record is not `RigidBody`.** It needs inverse mass and the world-space
   inverse inertia beside velocity. World inertia is derived from rotation and local inertia every
   step, so that derived record exists under either option. Option (b) does not remove the copy. It
   only moves where pose and velocity live.
2. **Change detection.** The solver writes velocity about 12 times per step.
   - If `RigidBody` were the solver column, there are two choices, and both lose. Either every write
     stamps a change tick, which is a hot-loop cost. Or `RigidBody` becomes untracked, and downstream
     systems (transform sync, render) lose `Changed<RigidBody>`.
   - Under (a), S6's writes stamp ticks only for awake dynamic bodies. That is exactly the set
     downstream must re-propagate, so sleeping bodies cost downstream nothing.
3. **What other engines do.** They converge on (a), each for its own reasons:
   - Jolt and PhysX keep a separate body store and sync only the active bodies.
   - Unity Physics rebuilds its world from ECS data each step.
   - avian added derived solver-body components for cache efficiency.
4. **The copy's cost.** It is O(awake bodies), runs in parallel (K4) and is bandwidth-bound. Arithmetic
   gives about 350 B per body per step: about 0.43 MB at 1240 bodies, which is tens of microseconds,
   and about 35 MB at 100k bodies, about 1.7 ms at 20 GB/s. These figures are arithmetic and have not
   been measured. That is why SP-1 remains the gate.

## Orchestrator decisions the design already assigned (section 14)

| Decision | Choice | Why, or what decides it |
|---|---|---|
| ~~RunCtx trampoline vs TLS-depth fallback (gang nesting rule)~~ | **Superseded by design rev 3** (P-§10.1): lanes are tickets on a per-pool `LaneBoard` that only `worker_main`'s top level probes. No `run_task` site carries a run context, so neither mechanism is built. (Originally decided here: trampoline.) | The TLS measurement (`docs/threadpool/RUSTC-198-WINDOWS-GNU-TLS.md`) is still the reason the board adds no per-task TLS read: under rustc >= 1.98 on `windows-gnu`, every `thread_local!` read takes two locked read-modify-writes on a global line plus `FlsSetValue`. |
| Broadphase default | **`Auto`** instead of `AllPairs` | Today's default runs a serial O(n²) sweep: 769,420 pairs and two `sqrt` per pair per step on the pyramid (design X-1). The 4096-body parallel gate is never reached. The pair set is bit-identical between arms (`systems.rs:275-276`). Lands at rung P1, gated by pair-set identity at W in {1, 8, 16} and by G-jolt. |
| Retiring the Rapier / Jolt-FFI allowances | **Retire** | Owner order (1): no allocator but the engine's own. Physics is in-house. |
| First performance rung | **Chosen by the R0 per-stage timing** (design §12) | Waits for the owner's quiet-machine word |
| Gang spin budget, `BatchingStrategy` for S1/S6, fusing S2+S3 into one gang, persistent incremental colouring | **Decided with numbers at the rung that introduces each** | Each needs its own timing |

## What this file does not do

- It does not edit the design. Rev 2 and any rev-3 patch appended after the second critique pass stay
  as written. The decisions here select among the options that design already lays out.
- It does not settle the engine-wide questions (render world, UI, input and the rest). Those come from
  the sibling study, `docs/unification/ENGINE-RUNTIME-ECS-DESIGN.md`, and are decided by the same
  criterion in its own decisions file.
