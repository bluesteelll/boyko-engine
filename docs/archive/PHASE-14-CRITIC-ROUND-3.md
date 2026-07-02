> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase 14a — Architecture Critic, Round 3 (focused confirmation) + resolution

**Round 3 verdict (on the §8 Round 2 patches P1-P5): REVISE** — 4/5 patches
(P1, P3, P4, P5) RESOLVED with no new soundness problem; **P2 defective**
(phantom `RawCommandQueue::apply` method; `raw()`/`RawCommandQueue` private to
`command_queue.rs`; calling the catch-free `apply_or_drop_queued_no_catch`
directly would discard the `catch_unwind` + `handle_panic_recovery(0)` survivor
re-absorb that W5/SAFETY-5/-6 require). + 1 LOW completeness note on P1.

**Resolution (applied to PHASE-14-OBSERVERS-PLAN-ROUND2.md §8): APPROVED-MET.**
The critic's exact conditional was: *"Once P2 is corrected (and P1's one-sentence
generalization added), all five patches resolve their issues with no new
soundness problem and the developer can start Wave 1."* Both corrections applied:

- **P2 corrected:** restored the `pub(crate) CommandQueue::apply_via_raw_twin(NonNull<EcsMaster>)`
  **associated function** (defined in `command_queue.rs`, keeping `raw()` /
  `RawCommandQueue` private) that mirrors `CommandQueue::apply`
  (command_queue.rs:203-251) — the `raw()` mint + single `catch_unwind` +
  `handle_panic_recovery(0)`. It takes `world: NonNull<EcsMaster>` (not `&mut`)
  and derives the queue internally, so there is **no `&mut self`(queue) receiver**
  to alias `&mut *world` (the sharper queue⊂world aliasing). The drain loop tests
  emptiness via a transient shared borrow, then calls the sibling. The phantom
  `raw_twin.apply(world_ptr)` line + the "raw twin already provides the walk"
  claim were deleted. SAFETY-5/-6 re-stated against the sibling.
- **P1 generalized:** stated the RAII `DeferredScopeGuard` applies at ALL FIVE
  bracket sites — the 3 direct-API methods + the 2 schedule `system.apply` sites
  (schedule.rs:339 / :516). The schedule sites are the highest panic risk (no
  schedule-level `catch_unwind`; the only catch is inside `CommandQueue::apply`
  at command_queue.rs:244), so the guard is held across `system.apply(world)` to
  survive a command panic propagating through the un-caught :339 site.

**Confirmed RESOLVED by Round 3 (unchanged):**
- P1 RAII guard — sound (matches CursorSync/InSystemRunGuard/DeferredEcsMaster
  raw-NonNull-across-`&mut` precedent); all `create_entity`/`create_entity_at`
  `Err` returns verified to precede the hook-fire point (nothing enqueued on Err);
  no double-decrement (`drop(scope)` consumes the guard).
- P3 firing matrix — `on_add` over `I\S`, `on_insert` over bundle set `I`,
  retained `S\I` fires nothing; `bundle_ids` captured in Step-2 closure (before
  `move_out_entity`); `MAX_BUNDLE_ARITY` parity with `bundle_slots`;
  `on_replace`-for-`I∩S` deferral is an acceptable documented scope decision.
- P4 bench targets — sufficient as a gate; proactive `#[cold]` helper hoist
  recommended (not required).
- P5 measure-and-pin — adequate.

**Rationale for proceeding without a 4th architecture-critic pass:** the critic
gave an explicit conditional approval and verified the surrounding facts (private
`raw()`, catch-free twin walk, catch location at :244-250). The P2 correction
uses exactly those verified facts and mirrors the existing Miri-validated
`CommandQueue::apply` almost verbatim (same `raw()` mint, same `catch_unwind`,
same `handle_panic_recovery(0)`), differing only in taking `world` as `NonNull`.
The **code-reviewer** will verify the actual `apply_via_raw_twin` + drain
implementation against real source during the review loop — a stronger check on
the realized code than re-reviewing plan prose.

**Status: plan APPROVED for implementation. Developer may start Wave 1.**
