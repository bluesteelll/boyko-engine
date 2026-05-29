# Architecture: Phase 14a — Component Lifecycle Hooks (Round 2 Revision)

> **Revision of** [`PHASE-14-OBSERVERS-PLAN.md`](PHASE-14-OBSERVERS-PLAN.md) in
> response to [`PHASE-14-CRITIC-ROUND-1.md`](PHASE-14-CRITIC-ROUND-1.md)
> (**NEEDS REWORK** — C1, C2, C3 + W1-W5 + O1-O3 + 5 open questions).
>
> This document is **self-contained but additive**: it states every revised
> decision in full and is read **alongside** the Round 1 plan. Where a Round 1
> §-number is unchanged (Q5 cold `HOOKS` table, the next-pass instinct, PRE-DROP
> firing, drop-order placement), it is preserved verbatim and not relitigated —
> the critic's "Positive (preserve these)" list is honored.
>
> Every internal claim is grounded against source at the cited `file:line`. All
> code blocks containing `unsafe` carry `// SAFETY:` discipline (CLAUDE.md #8).

---

## §0 — Answers to the critic's 5 open questions

**Q-A1 (drain ownership → ONE owner + depth counter).** There is exactly **one
drain owner** at the outermost apply boundary, plus a reentrancy **depth
counter** (`EcsMaster::hook_drain_depth: u32`). The "drain inside every command's
apply body" option from Round 1 §5.4 is **dropped**. Two `enter_deferred_scope()`
/ `exit_deferred_scope()` calls bracket the *whole* `system.apply(world)` call:
one around `apply_window_drain`'s `self.systems[i].system.apply(world)`
(schedule.rs:339) and one around the inline-exclusive path's
`self.systems[i].system.apply(world_ref)` (schedule.rs:516). After `exit` returns
the depth to 0, a single `world.drain_deferred_hook_queue()` runs. Direct-API
methods (`create_entity` ecs_master.rs:599, `create_entity_at` :697,
`delete_entity` :926) each `enter`/`exit` around their own body and drain once at
their end (depth is 0 there because no command-queue apply is on the stack).
Hooks fired *during* a drain enqueue into the **same** `deferred_hook_queue`; the
single outer `while !deferred_hook_queue.is_empty()` loop in
`drain_deferred_hook_queue` picks them up (re-entrant appends are visible because
the drain uses the raw-twin — Q-A4 — not `mem::take`-into-local). Net effect: the
nested-apply structure collapses to **one** `catch_unwind` level — the per-system
`CommandQueue::apply` (command_queue.rs:240-251) is the only catch on the stack
during the triggering command, and the drain runs strictly *after* it returns.

**Q-A2 (`get_component_mut` in 14a → DROP it).** `DeferredEcsMaster` exposes
**only** read-only and resource access: `get_component<T>(&self)`,
`resource<R>(&self)`, `resource_mut<R>(&mut self)`, `commands()` (deferred), and
`current_tick(&self)`. There is **no** `&mut`-into-archetype-storage method on the
view, so a hook **cannot even construct** an aliasing `&mut` into a component pool
buffer (`get_component_raw_mut` at ecs_master.rs:986-1021 is never reachable
through the view). This statically closes the C2 "user-violable `pub fn`" hole —
the non-aliasing obligation is no longer a *documented* contract a user can break,
it is a *missing method*. Mutable component access (Bevy's `DeferredWorld`
component-write surface) is deferred to **14b** behind a proven non-aliasing
story. The canonical patterns survive: "on_remove: decrement a counter" uses
`resource_mut`; "read the dying value" uses `get_component`.

**Q-A3 (migration-site aliasing → phased restructure of all 6 sites).** Every
hook-fire site is restructured to a uniform **phased shape**:
(1) read any dying / needed bytes + the component-id list into **owned/stack**
storage; (2) **drop every `world`-derived `&mut Archetype` / `&mut ComponentPool`**
— confine them to a closed lexical block and retain only raw `*mut` *by value*;
(3) mint `let world_ptr = NonNull::from(&mut *world)`; (4) fire hooks; (5)
re-resolve borrows; (6) continue (`drop_at` / `move_out_entity` / `remove_entity`).
At the mint point in step 3, **every** `world`-derived `&mut` is provably dead
(NLL: its last use was inside the step-1/2 block). This is realizable at all six
sites — Round 1's §9 SAFETY-1(b) "drop the `&mut Archetype` before firing" was
stated universally but only *sketched* at the spawn + despawn sites; Round 2
makes it concrete at the two migration sites too (§3.4, §3.5), which the critic
correctly flagged (C2) as holding `source`/`target` `&mut` live across the fire
point (migration_helpers.rs:189-190 live through :403; :438-439 live through
:519).

**Q-A4 (re-entrant drain mechanism → `RawCommandQueue` raw-twin).** The drain uses
the existing **raw-twin** pattern (`CommandQueue::raw()`, command_queue.rs:155-173)
that `apply` / `Drop` already use — **not** `mem::take`-into-local. The raw-twin
reads `bytes` / `cursor` / `panic_recovery` *by value as `NonNull`* via `&raw mut`
without materializing an intermediate `&mut` on those fields (command_queue.rs:166-172),
so survivors-on-panic semantics are preserved (W5) and re-entrant appends pushed
during the drain land in the **same** `bytes` allocation and are seen by the
single outer `while !is_empty()` loop. `mem::take`-into-local would (a) make
re-entrant appends invisible to the outer loop and (b) silently discard survivors
if the local's `apply` panicked (the local's `Drop` runs drop-glue on the
survivors, and the field was already emptied) — exactly the W5 hazard. The
raw-twin avoids both, and avoids field aliasing because `world.deferred_hook_queue`
is read through the twin's `NonNull<Vec<..>>` rather than through a `&mut` that
would alias the `&mut EcsMaster` passed to `apply`.

**Q-A5 (staleness in release → RELEASE-level scan check).** `register_component_hooks`
(cold, one-time) performs a **release-level** scan: if any *live* archetype
already contains `C`, it **panics** with a clear message (the flag for that
archetype was computed at construction from `HOOKS[id]` — §4.6 / archetype.rs:226-229 —
and would be stale, silently skipping the hook). The derive path is
**staleness-immune by construction**: hooks install inside the `OnceLock`-backed
`component_id()` (boyko_macros first-call), which always precedes the first
archetype containing the component (archetype construction calls `component_id()`
to get the ID). Hand-written `impl Component` (e.g. ecs_master.rs:2129-2137, which
hand-impls `component_id()` with no hook install) **must** register before first
spawn — the release check enforces it. Round 1's "debug-assert only; release
no-op" is **upgraded to a release panic** (the W3 fix): a silently dropped
lifecycle callback is too severe a correctness surprise for a feature whose entire
value is "the callback fires."

**Migration-remove observation (state explicitly).** At the pre-`drop_at` fire
point in `migrate_entity_remove`, the entity's `EntityInland` **still points at
SOURCE** (the repoint to target happens at migration_helpers.rs:518-519, *after*
`drop_at` at :505). Therefore the hook reads the **SOURCE (dying) row**. By
contrast, `migrate_entity_insert` fires `on_add` / `on_insert` **AFTER** migration
completes and `EntityInland` is repointed to target (migration_helpers.rs:402-403),
so those hooks read the **NEW target row** (Bevy parity: add/insert observe the
entity in its post-op location; remove observes the pre-op dying value). This
asymmetry is intentional and load-bearing for the test assertions (§8).

---

## §1 — Per-finding resolutions

### C1 — Single-owner drain + depth counter; step-by-step panic-interaction proof

**Revised design.** Drain ownership is the outermost apply boundary only
(Q-A1). The `deferred_hook_queue` is drained by `EcsMaster::drain_deferred_hook_queue`
(§2), called from exactly three kinds of site, each at `hook_drain_depth == 0`:

- `apply_window_drain` (schedule.rs:315-366), **after** the bracketed
  `system.apply(world)` (schedule.rs:339) returns and `exit_deferred_scope` has
  decremented the depth to 0.
- The inline-exclusive path (schedule.rs:495-531), **after** the bracketed
  `system.apply(world_ref)` (schedule.rs:516) returns.
- Each direct-API public method (`create_entity` / `create_entity_at` /
  `delete_entity`) at its own end.

Commands' `apply` bodies (`SpawnAtCommand`, `InsertCommand`, `RemoveCommand`,
`DespawnCommand`) **no longer drain**. They only *fire hooks* (which *enqueue*
into `deferred_hook_queue`). The redundant second drive the critic identified in
the `DespawnCommand::apply → delete_entity` chain (despawn_command.rs:32-43 →
ecs_master.rs:886-926) is eliminated by construction: `delete_entity` is called
from `DespawnCommand::apply` at `hook_drain_depth > 0` (the per-system
`CommandQueue::apply` is on the stack and bumped the depth via
`enter_deferred_scope`), so `delete_entity`'s own end-of-method
`drain_deferred_hook_queue` call observes `depth > 0` and **returns immediately**
without draining. The single drain happens once, at the outermost boundary, when
the per-system `CommandQueue::apply` has fully returned.

**The depth-gated drain (the load-bearing rule):**

```rust
// EcsMaster::drain_deferred_hook_queue (§2). The depth gate is the whole
// correctness story: drain runs ONLY when depth has returned to 0.
fn drain_deferred_hook_queue(&mut self) {
    if self.hook_drain_depth != 0 {
        return; // nested call (inside a CommandQueue::apply); the outermost owner drains.
    }
    debug_assert!(!boyko_threadpool::is_in_system_run()); // SAFETY-7 tripwire (W4)
    while !self.deferred_hook_queue.is_empty() {
        // raw-twin apply (Q-A4 / §2); re-entrant appends land in the same
        // `bytes` and are seen by the next loop turn.
        // ...
    }
}
```

**Step-by-step panic-interaction proof (the C1 crux).** Claim: a deferred-command
panic *during the drain* interacts with the existing single `catch_unwind` /
`CursorSync` machinery as **exactly one level** — no two-level recovery, no
stale-entity replay, no double-apply.

1. A per-system `CommandQueue::apply` is running (command_queue.rs:203-251). It
   minted its raw twin (`self.raw()`, :221) and entered the single `catch_unwind`
   (:244). `enter_deferred_scope` was called at the bracket around
   `system.apply(world)` (schedule.rs:339), so `hook_drain_depth == 1`.
2. A command body (say `DespawnCommand::apply`) fires `on_remove` for a dying
   component. The hook calls `ctx.commands().spawn(bundle)`, which `push`es a
   `SpawnAtCommand` into `world.deferred_hook_queue` (a *different* `CommandQueue`
   from the per-system one — different `bytes` allocation). **No** drain runs
   here: the command body does not drain (Round 2), and any `delete_entity`
   end-of-method drain sees `depth == 1` and returns.
3. The command body returns; the per-system walk continues; the walk completes;
   `CommandQueue::apply` returns Ok (command_queue.rs:244 took the non-`Err`
   path); the raw twin and `catch_unwind` frame are gone from the stack.
4. `system.apply(world)` returns to `apply_window_drain` (schedule.rs:339).
   `exit_deferred_scope` decrements `hook_drain_depth` to 0.
   `drain_deferred_hook_queue` is now called (§2): `depth == 0`, so it drains.
5. The drain applies the enqueued `SpawnAtCommand` via the raw-twin
   `apply_or_drop_queued_no_catch` wrapped in **one** `catch_unwind` (the same
   shape as command_queue.rs:240-251, but on `deferred_hook_queue`). **Critically,
   the per-system queue's `catch_unwind` is no longer on the stack** (step 3
   returned). So if the deferred command panics, the *only* `catch_unwind` that
   captures it is the drain's own. There is no outer per-system `catch_unwind` to
   double-capture into a second `handle_panic_recovery` on a different queue. The
   two applies were **never simultaneously live on the stack** (the drain is
   strictly after the per-system apply returned).
6. On a deferred-command panic, `handle_panic_recovery(0)` runs on
   `deferred_hook_queue` (start == 0, single-level — command_queue.rs:560-566
   re-absorbs survivors into `bytes` in the same call). Survivors are re-absorbed
   into `deferred_hook_queue.bytes`; the next turn of the `while !is_empty()` loop
   would re-walk them — **but** because the raw-twin (Q-A4) makes this the *same*
   queue and the *same* single-level recovery that Phase 12.5/12.6 already
   Miri-validated, the behavior is provably identical to a top-level
   `CommandQueue::apply` panic. There is no "deferred queue replays on the next
   frame against a stale entity" (the C1 hazard) because the drain re-absorbs and
   re-walks *within the same outermost boundary*, not across frames, and a
   re-absorbed survivor whose triggering entity is gone is no different from any
   command applied against a despawned entity — handled by the existing
   stale-entity `debug_assert` + release no-op (despawn_command.rs:36-41,
   insert_command.rs:56-63, remove_command.rs:69-76).

Therefore the panic story is **one** `catch_unwind` level: the per-system queue's
catch (for the triggering command) and the drain's catch (for deferred commands)
are sequential, never nested. This is the flecs-style "depth counter + single
drain at the outermost boundary" the critic recommended, mapped onto boyko's
existing raw-twin machinery untouched.

### C2 — Reduced read-only view + restructured control flow at all 6 sites

**Revised view (Q-A2).** Drop `get_component_mut`. The view exposes only
read-only `get_component<T>`, `resource<R>`, `resource_mut<R>`, deferred
`commands()`, and `current_tick`. See §2 for the exact Rust signature block. With
no `&mut`-into-storage method, a hook cannot construct an aliasing `&mut` into a
component pool buffer — the C2 "safe-looking `pub fn` that produces UB" is gone
(it is now a missing method, not a documented obligation; CLAUDE.md #8 honored).

**Restructured control flow (Q-A3) + per-site liveness argument.** Full phased
pseudocode for each site is in §3; the per-site one-line liveness assertion:

| Site | Liveness assertion at the `NonNull::from(&mut *world)` mint point |
|---|---|
| spawn-deferred (spawn_at_command.rs ~242→247) | the closure's per-invocation `&mut *archetype_ptr` (spawn_at_command.rs:223) already dropped at closure return (:231); the post-Step-6 `&mut Archetype` reborrow is *not taken* — only `archetype_ptr` (`*mut`, Copy) survives. |
| spawn-direct (ecs_master.rs:578-580 / :685-687) | the `&mut Archetype` is block-scoped (`let pushed = { ... }` :567-580) and dropped at the block close before `register_entity_with_ptr` (:599); only `archetype_ptr` survives. |
| insert-in-place (insert_command.rs:138) | the per-invocation `&mut *archetype_ptr` (:138) drops at each closure-call return; hooks fire *after* the `for_each_component_bytes` loop, holding only `archetype_ptr`. |
| insert-migration (migration_helpers.rs:189-403) | `source`/`target` (`&mut`, :189-190) confined to a Phase-1 block; only `source_ptr`/`target_ptr` (`*mut`, Copy) survive into the fire point (§3.4). |
| remove-migration (migration_helpers.rs:438-519) | `source`/`target` (`&mut`, :438-439) confined to a Phase-1 block; only `source_ptr`/`target_ptr` survive into the fire point (§3.5). |
| despawn (ecs_master.rs:905-906) | the `&mut Archetype` (:905) is confined to a read-only-id block; only `inland.archetype_ptr()` (`*mut`) survives into the fire point. |

At every site the rule is identical: the only `world`-derived value alive across
the hook fire is a raw `*mut`/`NonNull` *by value*; no `&mut Archetype` or
`&mut ComponentPool` derived from `world` is live, so minting
`NonNull::from(&mut *world)` and re-deriving access through the read-only view
does not violate stacked/tree borrows against any live reborrow.

### C3 — Move the load-bearing 0% bench gate to Wave 4 (real dispatch)

**Revised design.** Round 1's Wave 1 Step 4 benched `if !flags.is_empty() { /* empty */ }`
while `flags` is *always* `empty()` until Wave 2 — a DCE target that proves
nothing (the critic is correct). Round 2:

- **Wave 1 is reduced** to "field added + layout assertions pass" (the two `const _`
  tripwires of §2/W2 compile; `offset_of!(Archetype, columns) == 0` still holds at
  archetype.rs:170). No bench gate in Wave 1.
- **The load-bearing 0%-regression bench gate moves to Wave 4**, where dispatch is
  **real**: the gate is a *populated-flags* runtime `u16` (OR-computed from the
  cold `HOOKS` table at construction — §4.6) guarding a `#[cold] #[inline(never)]`
  *real* trigger fn. The bench measures an honest load + `test`/`jz` +
  cold-call-not-taken in the no-hook case (`flags.is_empty()` true at runtime, the
  optimizer cannot prove it zero because it comes from a cold table). 
- **Verification via `cargo asm`** on a **real Wave-4 site** (e.g. the
  `SpawnAtCommand::apply` post-Step-6 gate), confirming the not-taken arrangement
  is one load + one branch + a cold un-entered call — **not** a Wave-1 empty block.

The acceptance bar (Round 1 §1 table) is unchanged in *target* (0% measurable
regression vs pre-Phase-14a baseline, ±5% criterion noise, Phase 10 precedent);
only the *wave* where it is gated changes.

### W1 — Stack `[ComponentId; MAX_COMPONENTS]` buffer for despawn (no per-despawn alloc)

**Revised design.** Round 1's `archetype.component_ids().to_vec()` (Round 1 §4.5)
heap-allocates on **every** despawn of an entity in any archetype with a
remove/despawn hook — the *intended steady state* for the canonical "on_remove
counter" pattern, violating CLAUDE.md #1/#5. Round 2 replaces it with a **stack
buffer** mirroring the `[ComponentId; MAX_MIGRATION_COLUMNS]` pattern already in
the codebase (migration_helpers.rs:69, `MAX_MIGRATION_COLUMNS = MAX_COMPONENTS`
at :39):

```rust
// Despawn site (ecs_master.rs delete_entity, pre-:906). Entered ONLY when
// flags is non-empty (cold). The ≤32-typical archetype width means only the
// touched prefix [0..n) is written — no full 512-entry memset.
let flags = { /* read-only block: copy out the u16, drop the &mut */ };
if !flags.is_empty() {
    let mut id_buf = [ComponentId(0); MAX_COMPONENTS]; // stack, ~4 KB, uninit-cheap
    let n = { /* fill id_buf[..n] from component_ids() under a short shared borrow */ };
    // ... drop the shared borrow, mint world_ptr, fire hooks over &id_buf[..n] ...
}
```

The no-hook hot path **never enters the block** (the `flags.is_empty()` guard is
the same one C3 benches as free). The cold path writes only the `[0..n)` prefix
(`n ≤ ~32` typical, `≤ 512` worst case), so there is no full-array memset cost.
This is resolved *in the plan*, not deferred to a follow-up (the critic's
explicit demand). The same stack buffer serves the hooked-despawn/spawn paths
(§3.6).

### W2 — Correct the "lands in padding, zero growth" claim + two hard tripwires

**Revised design.** The Round 1 §3.3 claim that `flags: u16` "slots into existing
padding... zero size growth" is **FALSE** and is corrected. Grounded in source:
`signature: ArchetypeSignature` (archetype.rs:145) embeds `ComponentMask`, which
is `#[repr(align(32))]` (component_mask.rs:7) with an 8×`BitSet<u64>` = 64 B body.
Because the signature is 32-aligned and a multiple of 32 B in size, the offset
**after** it is already 8-aligned, so `arena: *const Arena` (archetype.rs:152)
today starts with **zero padding**. Inserting `flags: u16` before `arena`
therefore adds **2 B + 6 B realign = +8 B** (one pointer-slot), not zero. This is
functionally harmless (`Archetype` is ~8 KB — dominated by `columns: [Column; 512]`
= 512 × 16 B = 8192 B at archetype.rs:123) but the *stated justification* was
wrong and invites a bad future "optimization."

**Corrected field placement** (real Round 1 §3.3 sketch had phantom fields; the
actual order is grounded at archetype.rs:109-163):

```rust
#[repr(C)]
pub struct Archetype {
    pub(crate) columns: [Column; MAX_COMPONENTS], // offset 0 — UNCHANGED, asserted :170
    pub(crate) id: ArchetypeId,
    pub(crate) component_pools: ComponentPoolBundle,
    pub(crate) current_index: usize,
    pub(crate) signature: ArchetypeSignature,      // #[repr(align(32))] inner mask
    pub(crate) flags: ArchetypeFlags,              // NEW — u16; adds +8 B (NOT zero — W2)
    pub(crate) arena: *const Arena,
    pub(crate) component_ids: Vec<ComponentId>,
    pub(crate) entity_ids: Vec<EntityId>,
}

// STILL HOLDS — flags is placed after signature, before arena; columns stays first.
const _: () = assert!(std::mem::offset_of!(Archetype, columns) == 0); // archetype.rs:170

// TRIPWIRE 1 (W2): hard size assertion. Value to be MEASURED on the target
// during Wave 1 Step 2 and pinned here (e.g. via `cargo test` printing
// size_of::<Archetype>() once, then frozen). Placeholder until measured:
const _: () = assert!(std::mem::size_of::<Archetype>() == /* MEASURED */ 0 + std::mem::size_of::<Archetype>());

// TRIPWIRE 2 (W2): the ComponentLayout 56 B assertion is DOC-ONLY today
// (component_registry.rs:90 — a comment, not a const). It must be ADDED:
const _: () = assert!(std::mem::size_of::<ComponentLayout>() == 56);
```

> **Note on TRIPWIRE 1**: the Round 1 plan asserted a *concrete* zero-growth
> value; since W2 shows growth is +8 B, the exact `size_of::<Archetype>()` must be
> **measured** on the target in Wave 1 Step 2 and the literal pinned. The
> developer replaces the placeholder with the measured constant; the assertion
> then guards against accidental layout drift. TRIPWIRE 2 is a real *addition*
> (the critic verified component_registry.rs:90 is doc-only).

### W3 — Release-level staleness check + derive-immunity (Q-A5)

**Revised design.** Per Q-A5: `register_component_hooks` performs a **release**
scan (cold, one-time) and **panics** if any live archetype already hosts `C`
(its flag, computed at archetype.rs:226-229 from `HOOKS[id]`, would be stale). The
derive path is staleness-immune by construction (hooks install at `component_id()`
first-call, before any archetype). Hand-written `impl Component`
(ecs_master.rs:2129-2137) + the Phase 8.5 slab cache path are covered: a
hand-impl + runtime `register_component_hooks` after an archetype exists now
**panics in release** rather than silently skipping. Round 1's "silently skipped
in release" is no longer an accepted outcome (the critic's W3 demand: "Pick one;
don't leave silently-skipped as an accepted release outcome").

### W4 — SAFETY-7: hooks fire with `IN_SYSTEM_RUN == false` + tripwire

**Revised design.** Add **SAFETY-7** (§5) documenting that hooks fire with
`IN_SYSTEM_RUN == false`, **verified** against source: `InSystemRunGuard::enter`
(tls.rs:152-159) is invoked only around `System::run_unsafe` — in the concurrent
path the guard is created at schedule.rs:605 and **dropped at :623, *before*
completion is published** and long before the dispatcher's `system.apply(world)`
(schedule.rs:339); in the inline-exclusive path `run_unsafe` (schedule.rs:511) is
*not* wrapped in a guard at all, and `apply` (schedule.rs:516) runs outside any
guard. Therefore a hook's deferred command that triggers an arena allocation
(spawn into a new archetype) does **not** trip ALLOC1's
`debug_assert!(!IN_SYSTEM_RUN)` (tls.rs:37, asserted in `Arena::allocate_*`) — a
positive that Round 1 left unstated. Add a tripwire at the top of
`drain_deferred_hook_queue`:

```rust
// SAFETY-7 (W4): hooks + their deferred commands run OUTSIDE the system-body
// allocation-discipline window. Verified: InSystemRunGuard wraps only
// run_unsafe (tls.rs:152-159; schedule.rs:605 created, :623 dropped before
// apply). If a future refactor moved `apply` inside the guard, this fires.
debug_assert!(!boyko_threadpool::is_in_system_run(),
    "SAFETY-7: hook drain must run with IN_SYSTEM_RUN == false");
```

### W5 — Folded into the raw-twin (Q-A4)

**Revised design.** Round 1 §5.4 proposed `mem::take`-the-bytes-into-a-local,
apply, restore capacity. The critic correctly noted this **changes panic
semantics**: if the local's `apply` panics, the local is dropped during unwind,
its `Drop` (command_queue.rs:693-721) runs drop-glue on survivors which are then
**gone** (not re-absorbed into `world.deferred_hook_queue`, which `mem::take` left
empty) — a silent discard of surviving deferred commands. Round 2 **drops the
`mem::take` dance entirely** and uses the **raw-twin** (Q-A4 /
`CommandQueue::raw()`, command_queue.rs:155-173): `drain_deferred_hook_queue`
runs `deferred_hook_queue.apply(world)` shape via the raw twin minted from
`&mut self.deferred_hook_queue`, exactly mirroring `CommandQueue::apply`
(command_queue.rs:203-251) including its single `catch_unwind` +
`handle_panic_recovery(0)` re-absorb (:560-566). Survivors re-absorb into the same
`bytes`; re-entrant appends are visible to the outer loop. **No field aliasing**:
the raw twin reads `bytes`/`cursor`/`panic_recovery` as `NonNull` via `&raw mut`
*without* a `&mut` on those fields (command_queue.rs:166-172), and `world` is
passed as a separate `NonNull<EcsMaster>` (`world.deferred_hook_queue` and the
`world_ptr` do not alias because the twin accesses the queue's heap buffers, not
through the `&mut EcsMaster`). See §2 for the shape and §5 SAFETY-5.

### O1 — `ComponentHooks: Send + Sync` note

`static HOOKS: [OnceLock<ComponentHooks>; MAX_COMPONENTS]` (§2) requires
`ComponentHooks: Send + Sync`. `ComponentHooks` is five `Option<HookFn>` fields
(plain `unsafe fn` pointers — `fn` pointers are unconditionally `Send + Sync`), so
the auto-derived `Send + Sync` holds with no `unsafe impl` needed. One-line note
matches the rigor every other `static` gets (mirrors the `LAYOUTS` precedent at
component_registry.rs:141, where `ComponentLayout` is `Copy` + fn-pointer-only).

### O2 — Declarative macro for the 5 `trigger_on_*` fns + `#[cold]` verification

The five near-identical `trigger_on_*` fns (Round 1 §4.1, byte-identical modulo
the `hooks.on_X` field selected) become a **declarative macro** (Phase 10
precedent — the `set_table_*` migration used the same macro-collapse technique).
The macro expands one `#[cold] #[inline(never)]` fn per kind, selecting the field:

```rust
macro_rules! define_trigger {
    ($name:ident, $field:ident, $bit:expr) => {
        /// Fire `$field` for `component_id` on `entity`. Cold: only called
        /// when the archetype's `$bit` is set (the caller's gate).
        #[cold]
        #[inline(never)]
        pub(crate) fn $name(world: NonNull<EcsMaster>, component_id: ComponentId, entity: Entity) {
            if let Some(hooks) = component_registry::get_hooks(component_id)
                && let Some(f) = hooks.$field
            {
                // SAFETY (SAFETY-1, -4): minted from the &mut EcsMaster the
                //   outermost apply holds; apply-window-only; the read-only
                //   view withholds structural + mutable-component ops (Q-A2);
                //   ctx.entity is live.
                let view = unsafe { DeferredEcsMaster::from_world(world) };
                let ctx = HookContext { entity, component_id };
                // SAFETY (HookFn contract): see DeferredEcsMaster + apply window.
                unsafe { f(view, ctx); }
            }
        }
    };
}
define_trigger!(trigger_on_add,     on_add,     ArchetypeFlags::ON_ADD_HOOK);
define_trigger!(trigger_on_insert,  on_insert,  ArchetypeFlags::ON_INSERT_HOOK);
define_trigger!(trigger_on_replace, on_replace, ArchetypeFlags::ON_REPLACE_HOOK);
define_trigger!(trigger_on_remove,  on_remove,  ArchetypeFlags::ON_REMOVE_HOOK);
define_trigger!(trigger_on_despawn, on_despawn, ArchetypeFlags::ON_DESPAWN_HOOK);
```

Verify via `cargo asm` that `#[cold]` actually demotes each fn into a cold section
(not merely `inline(never)`) — a Wave-4 check alongside the C3 gate verification.

### O3 — Insert-replace closure placement covered by C2

The Q7 `on_replace` + `on_insert` placement at the live-borrow
`for_each_component_bytes` closure (insert_command.rs:138, where a per-invocation
`&mut *archetype_ptr` is reborrowed) is **subject to the C2 restructure**, not
independently settled. Per §3.3 (insert-in-place site): the hooks fire *outside*
the closure loop, after the last `&mut *archetype_ptr` invocation has dropped — so
the closure's per-call `&mut Archetype` is provably dead when `world_ptr` is
minted. The per-component `on_replace`-pre-`drop_at` / `on_insert`-post-`write_at`
*ordering* (Round 1 Q7) is preserved by firing per component but reading the value
through the read-only view at the fire point (no `&mut` into the pool).

---

## §2 — Revised data structures

### §2.1 Reduced `DeferredEcsMaster<'w>` surface (Q-A2 / C2)

```rust
/// Restricted READ-ONLY view of `EcsMaster` handed to lifecycle hooks (14a).
///
/// Exposes component READS, resource access, a deferred-command handle, and
/// the current tick — and statically WITHHOLDS (a) every structural-change
/// method and (b) every `&mut`-into-archetype-storage method. A hook firing
/// during the outermost apply therefore cannot construct an aliasing `&mut`
/// into a component pool buffer (C2 closed; §5 SAFETY-1). Mutable component
/// access is deferred to 14b.
///
/// # Why a wrapper, not `Deref`
/// There is intentionally NO `Deref<Target = EcsMaster>` — that would leak the
/// full (structural + mutable) API. The view forwards ONLY the safe subset.
#[repr(transparent)]
pub struct DeferredEcsMaster<'w> {
    // Raw NonNull, NOT &mut, so the outermost apply's *mut reborrows are not
    // invalidated under Tree Borrows. Minted from the same &mut EcsMaster the
    // apply holds, AFTER every world-derived &mut Archetype is dead (§3).
    world: NonNull<EcsMaster>,
    _marker: PhantomData<&'w mut EcsMaster>,
}

impl<'w> DeferredEcsMaster<'w> {
    /// SAFETY: `world` points at a live EcsMaster the caller holds exclusively,
    ///   and no `world`-derived `&mut Archetype`/`&mut ComponentPool` is live.
    pub(crate) unsafe fn from_world(world: NonNull<EcsMaster>) -> Self {
        Self { world, _marker: PhantomData }
    }

    // ── EXPOSED (sound during the outermost apply) ───────────────────────
    /// Read a component of a (possibly different) entity. Read-only: resolves a
    /// fresh `*const` via `get_component_raw` (ecs_master.rs:943-980) per call.
    pub fn get_component<T: Component>(&self, e: Entity) -> Option<&T>;
    /// Read a resource. Resources live OUTSIDE archetype storage.
    pub fn resource<R: Resource>(&self) -> Option<&R>;
    /// Mutate a resource. Resources live OUTSIDE archetype storage — never
    /// aliases the apply's component writes (the canonical on_remove pattern).
    pub fn resource_mut<R: Resource>(&mut self) -> Option<&mut R>;
    /// Enqueue a structural command into the world-resident deferred queue.
    /// Drained at the OUTERMOST boundary (Q-A1) — never inline.
    pub fn commands(&mut self) -> DeferredCommands<'_>;
    /// Read the current change-detection tick.
    pub fn current_tick(&self) -> Tick;

    // ── WITHHELD (NOT forwarded — compile error if a hook tries) ─────────
    //   get_component_mut / set_component_raw (no &mut into storage — Q-A2);
    //   create_entity / create_entity_at / delete_entity / spawn_* (structural);
    //   register_component_hooks (register-before-use — §W3/Q-A5).
}
```

### §2.2 Raw-twin `drain_deferred_hook_queue` shape (Q-A4 / W5)

```rust
impl EcsMaster {
    /// Drain the world-resident deferred-hook queue at the OUTERMOST apply
    /// boundary (Q-A1). Re-entrant: hooks fired during the drain enqueue into
    /// the SAME queue; the `while !is_empty()` loop picks them up because the
    /// raw twin (Q-A4) makes appends visible (unlike `mem::take`-into-local).
    fn drain_deferred_hook_queue(&mut self) {
        // Q-A1: only the outermost owner drains; nested calls (inside a
        // CommandQueue::apply at depth > 0) return immediately.
        if self.hook_drain_depth != 0 {
            return;
        }
        // SAFETY-7 (W4): verified IN_SYSTEM_RUN == false at every fire site.
        debug_assert!(!boyko_threadpool::is_in_system_run(),
            "SAFETY-7: hook drain must run with IN_SYSTEM_RUN == false");

        // Split the borrow: take the queue field's raw twin, pass `self` as a
        // separate NonNull. `world.deferred_hook_queue` is reached through the
        // twin's NonNull<Vec<..>> (no &mut on the field), so no aliasing with
        // `world_ptr` (§5 SAFETY-5).
        while !self.deferred_hook_queue.is_empty() {
            // Mirrors CommandQueue::apply (command_queue.rs:203-251): single
            // catch_unwind + handle_panic_recovery(0) re-absorb (:560-566).
            // The raw twin (CommandQueue::raw, :155-173) reads bytes/cursor/
            // panic_recovery as NonNull via `&raw mut` (no intermediate &mut).
            let world_ptr = NonNull::from(&mut *self);
            // SAFETY (SAFETY-5): the twin is the sole accessor of the queue's
            //   fields for this call; `world_ptr` accesses EcsMaster's OTHER
            //   fields. Re-absorbed survivors stay in the same `bytes`.
            unsafe { self.deferred_hook_queue.apply_via_raw_twin(world_ptr); }
        }
    }

    /// RAII brackets around the outermost `system.apply` (schedule.rs:339, :516)
    /// and each direct-API method body. Increment on enter, decrement on exit;
    /// the drain runs only when depth returns to 0.
    #[inline] fn enter_deferred_scope(&mut self) { self.hook_drain_depth += 1; }
    #[inline] fn exit_deferred_scope(&mut self)  { self.hook_drain_depth -= 1; }
}
```

> `CommandQueue::apply_via_raw_twin(NonNull<EcsMaster>)` is a thin internal
> sibling of the existing `apply(&mut EcsMaster)` (command_queue.rs:203) that
> takes the world as `NonNull` instead of `&mut` — needed because the queue is a
> *field* of `world`, so a `&mut EcsMaster` + `&mut self.deferred_hook_queue`
> would alias. It reuses `self.raw()` (:155) and the identical single-catch walk.

### §2.3 Depth counter + `deferred_hook_queue` field on `EcsMaster`

```rust
// NEW fields on EcsMaster. Grounded against the real field block
// (ecs_master.rs:116-252) — declared AFTER `arena` (Box<Arena>, :220) so any
// arena-derived command bytes are dropped before the arena slab, mirroring the
// `query_state_cache` C5 placement (:251) the critic praised.

/// World-resident channel a hook's DeferredCommands enqueues into. Reachable
/// via &mut EcsMaster (which the outermost apply holds), unlike the
/// borrow-frozen per-system Commands queue. Lazy: Vec::new() is alloc-free
/// (command_queue.rs:87) until first push.
pub(crate) deferred_hook_queue: CommandQueue,

/// Reentrancy depth (Q-A1 / C1). Bumped by enter_deferred_scope around the
/// outermost `system.apply` (schedule.rs:339, :516) and each direct-API body;
/// the single drain runs only at depth 0. `u32` is ample (hook chains are
/// shallow; a runaway is a user-error, same class as infinite recursion).
pub(crate) hook_drain_depth: u32,
```

### §2.4 `ComponentHooks` / `HookFn` / `HookContext` / `ArchetypeFlags` (preserved from Round 1)

Unchanged from Round 1 §3.2 / §3.3 (the critic's Q5/Q4 positives), with the O1
`Send + Sync` note added and the W2-corrected `Archetype` placement (§1-W2):

```rust
pub type HookFn = unsafe fn(DeferredEcsMaster<'_>, HookContext);

#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct ComponentHooks {           // 40 B, in HOOKS[id] (cold), never hot.
    pub on_add:     Option<HookFn>,   // fn pointers ⇒ Send + Sync (O1).
    pub on_insert:  Option<HookFn>,
    pub on_replace: Option<HookFn>,
    pub on_remove:  Option<HookFn>,
    pub on_despawn: Option<HookFn>,
}

#[derive(Clone, Copy)] #[repr(C)]
pub struct HookContext { pub entity: Entity, pub component_id: ComponentId } // 16 B

#[derive(Clone, Copy, Default, PartialEq, Eq)] #[repr(transparent)]
pub struct ArchetypeFlags(u16);       // 5 hook bits + 11 spare (14b observers).
// bit consts ON_ADD_HOOK..ON_DESPAWN_HOOK, contains/insert/is_empty — as Round 1 §3.3.

// Parallel cold table (Q5 — preserved). Keeps ComponentLayout at 56 B.
static HOOKS: [OnceLock<ComponentHooks>; MAX_COMPONENTS] =
    [const { OnceLock::new() }; MAX_COMPONENTS]; // mirrors LAYOUTS (component_registry.rs:141)
```

---

## §3 — Restructured control flow for all 6 hook-fire sites

Uniform phased shape (Q-A3): **read-into-owned → drop borrows → mint world ptr →
fire → re-resolve → continue**. Each site ends with a one-line borrow-liveness
assertion.

### §3.1 Spawn-deferred (`SpawnAtCommand::apply`, spawn_at_command.rs:106-248)

```rust
// AFTER Step 6 bookkeeping (:241-242) and Step 7 register_entity_with_ptr (:247).
// The closure's per-invocation `&mut *archetype_ptr` (:223) dropped at :231;
// we hold only `archetype_ptr` (*mut, Copy) and `entity` (Copy).
let flags = unsafe { (*archetype_ptr).flags };           // one u16 load; no &mut taken
if !flags.is_empty() {
    let world_ptr = NonNull::from(&mut *world);          // MINT: no world-derived &mut live
    // Ordering (SAFETY-2): ALL on_add, THEN ALL on_insert (Bevy bundle order).
    if flags.contains(ArchetypeFlags::ON_ADD_HOOK) {
        let ids = unsafe { (*archetype_ptr).component_ids.as_slice() }; // shared, transient
        for &cid in ids { trigger_on_add(world_ptr, cid, entity); }
    }
    if flags.contains(ArchetypeFlags::ON_INSERT_HOOK) {
        let ids = unsafe { (*archetype_ptr).component_ids.as_slice() };
        for &cid in ids { trigger_on_insert(world_ptr, cid, entity); }
    }
}
// NO drain here (Q-A1 / C1): the outermost boundary drains after this returns.
```

**Liveness:** at the `world_ptr` mint, the only live `world`-derived value is
`archetype_ptr` (`*mut`, Copy); the Step-5 closure's `&mut Archetype` (:223) and
the Step-6 field writes (:241-242) are dropped. ✓

### §3.2 Spawn-direct (`create_entity` ecs_master.rs:567-599 / `create_entity_at` :679-697)

```rust
// AFTER the block-scoped `let pushed = { let archetype = &mut *archetype_ptr; ... }`
// (:567-580 / :679-687) and register_entity_with_ptr (:599 / :697). The &mut
// Archetype was confined to the block and is dropped; only archetype_ptr survives.
self.enter_deferred_scope();                             // direct-API bracket (Q-A1)
let flags = unsafe { (*archetype_ptr).flags };
if !flags.is_empty() {
    let world_ptr = NonNull::from(&mut *self);
    /* on_add-all then on_insert-all over (*archetype_ptr).component_ids — as §3.1 */
}
self.exit_deferred_scope();
self.drain_deferred_hook_queue();                        // depth==0 here ⇒ drains
```

**Liveness:** the `&mut Archetype` is block-scoped (`let pushed = { ... }`,
ecs_master.rs:567-580) — dead before the gate. ✓

### §3.3 Insert-in-place (`apply_replace_in_place`, insert_command.rs:97-173)

```rust
// AFTER the for_each_component_bytes loop (:132-169). Per Q7, per-component
// on_replace fired PRE-overwrite and on_insert POST-overwrite — but the &mut
// pool reborrow (:138) is confined to the loop. We re-do a SECOND pass that only
// READS through the view (no &mut into pool):
//
// Round 2 ordering realization: the loop at :132 still does drop_at/write_at
// per component (the value churn). on_replace must observe the OLD value and
// on_insert the NEW. Since the view is read-only, we capture the needed dying
// snapshot into owned storage BEFORE the loop, fire on_replace from it after
// dropping borrows, run the existing overwrite loop, then fire on_insert.
let flags = unsafe { (*archetype_ptr).flags };
if flags.contains(ArchetypeFlags::ON_REPLACE_HOOK) {
    // Phase 1: read dying bytes for hooked components into owned storage while
    // a short &Archetype is live; drop it before minting world_ptr.
    /* snapshot via get_pool(cid).unit_ptr(row) into a stack/owned buffer */
}
// ... existing overwrite loop (:132-169) runs here (its &mut *archetype_ptr per
//     invocation drops at each call) ...
if !flags.is_empty() {
    let world_ptr = NonNull::from(&mut *world);          // MINT: closure &mut dead
    // on_replace (from the pre-loop snapshot semantics) THEN on_insert per comp;
    // on_add NOT fired (component already present — Q7).
}
```

**Liveness:** the per-invocation `&mut *archetype_ptr` (insert_command.rs:138)
drops at each closure-call return; at the `world_ptr` mint the loop has ended and
only `archetype_ptr` survives. ✓ (O3 resolved here.)

### §3.4 Insert-migration (`migrate_entity_insert`, migration_helpers.rs:151-404) — DETAIL

The critic's C2 core: `source: &mut Archetype` (:189) and `target: &mut Archetype`
(:190) are live through :403. Round 2 confines them to a **Phase-1 block** and
hoists the `EntityInland` repoint INTO it so the entity is fully in target before
hooks fire (Bevy parity: add/insert observe the NEW row — §0).

```rust
// PHASE 1 — all &mut Archetype work confined here (mirrors existing :189-403):
let (target_ptr, new_row, added_mask) = {
    let source: &mut Archetype = unsafe { &mut *source_ptr }; // :189
    let target: &mut Archetype = unsafe { &mut *target_ptr }; // :190

    // compute_added_mask: scan the ≤512-id space for "target ids NOT in source"
    // while source/target are still &mut-live; returns a BY-VALUE bitset. On the
    // cold #[inline(never)] migration path (:149-150) this is dwarfed by the
    // retained-byte memcpy (:228 from_raw_parts → :376 create_entity_with_ticks).
    let added_mask = compute_added_mask(source, target);

    // ... existing retained-collection (:198-247), bundle-collection (:255-277),
    //     merge (:285-354), create_entity_with_ticks (:376), move_out_entity (:387) ...

    // Step 6 repoint (:402-403) HOISTED INTO this block — touches
    // world.entity_master (NOT source/target), so it is hoistable. After this,
    // the entity is fully in target.
    world.entity_master.entities_inland[entity.id().0] =
        EntityInland::new(target_ptr, new_row, entity.generation());

    (target_ptr, new_row, added_mask)
    // <-- source/target &mut DROP here (block close).
};

// PHASE 2 — fire hooks; the entity is in target, repointed; source/target &mut dead.
let flags = unsafe { (*target_ptr).flags };
if !flags.is_empty() {
    let world_ptr = NonNull::from(&mut *world);          // MINT: source/target dead
    let target_ids = unsafe { (*target_ptr).component_ids.as_slice() };
    if flags.contains(ArchetypeFlags::ON_ADD_HOOK) {
        for &cid in target_ids {
            if added_mask.contains(cid) {                // newly-added only
                trigger_on_add(world_ptr, cid, entity);
            }
        }
    }
    if flags.contains(ArchetypeFlags::ON_INSERT_HOOK) {
        for &cid in target_ids { trigger_on_insert(world_ptr, cid, entity); } // all bundle comps
    }
}
```

**Liveness:** `source`/`target` (`&mut`, migration_helpers.rs:189-190) are
confined to the Phase-1 block and dropped at its close; the repoint (:402-403)
touches `world.entity_master`, not the archetypes, so hoisting it in is sound; at
the `world_ptr` mint only `target_ptr` (`*mut`, Copy) survives. ✓ Hooks read the
NEW target row (§0 asymmetry). ✓

### §3.5 Remove-migration (`migrate_entity_remove`, migration_helpers.rs:412-520) — DETAIL

`on_replace` + `on_remove` for `C` fire **PRE-`drop_at`** (migration_helpers.rs:505)
so the hook reads the dying SOURCE value (§0). The critic's dual-presence note is
addressed: Phase 2 has a momentary window where the entity is in **both** the
source row (`C` live) and the target row (`C` absent), with `EntityInland` still
pointing at SOURCE.

```rust
// PHASE 1 — confine source/target &mut; collect retained + push to target.
let removed_id = C::component_id();
let source_row;
{
    let source: &mut Archetype = unsafe { &mut *source_ptr }; // :438
    let target: &mut Archetype = unsafe { &mut *target_ptr }; // :439
    source_row = inland.unit_index() as usize;                // :435
    // ... existing retained-collection (:445-474) + create_entity_with_ticks
    //     (:488). The entity now exists in BOTH source (C live) and target
    //     (C absent); EntityInland STILL points at SOURCE.
    // <-- source/target &mut DROP here.
}

// PHASE 2 — fire PRE-drop hooks reading the SOURCE (dying) value (§0).
let flags = unsafe { (*source_ptr).flags };
if flags.contains(ArchetypeFlags::ON_REPLACE_HOOK) || flags.contains(ArchetypeFlags::ON_REMOVE_HOOK) {
    let world_ptr = NonNull::from(&mut *world);          // MINT: source/target dead
    // EntityInland points at SOURCE ⇒ get_component::<C> via the view reads the
    // dying source bytes (the consistent readable snapshot below).
    if flags.contains(ArchetypeFlags::ON_REPLACE_HOOK) { trigger_on_replace(world_ptr, removed_id, entity); }
    if flags.contains(ArchetypeFlags::ON_REMOVE_HOOK)  { trigger_on_remove(world_ptr, removed_id, entity); }
}

// PHASE 3 — re-resolve &mut source; drop C once; move_out; repoint to target.
{
    let source: &mut Archetype = unsafe { &mut *source_ptr };
    let removed_pool = source.component_pools_mut().get_pool_mut(removed_id).expect("..");
    // SAFETY (C5): source_row < count; &mut source exclusive; slot uninit after.
    unsafe { removed_pool.drop_at(source_row); }          // :505 — drops C ONCE
    match source.move_out_entity(InlandPoolId(source_row)) { /* :508-516 — swap-remove, no drop */
        RemoveOutcome::Swapped { moved_entity } => {
            // RemoveOutcome::Swapped fixup re-resolves &mut source AFTER the hook.
            if let Some(slot) = world.entity_master.entities_inland.get_mut(moved_entity.0) {
                slot.set_unit_index(source_row as u32);   // :511-513
            }
        }
        RemoveOutcome::Last => {}
        RemoveOutcome::PoolFailure => panic!("invariant: source removal must succeed"),
    }
}
world.entity_master.entities_inland[entity.id().0] =
    EntityInland::new(target_ptr, new_row, entity.generation()); // :518-519
```

**Dual-presence soundness (the §3.5 residual the critic flagged).** The Phase-2
window where the entity is in both rows is a **consistent readable snapshot, no
double-free**: (1) `drop_at` (Phase 3, migration_helpers.rs:505) drops `C` exactly
once; (2) `move_out_entity` (:508) swap-removes the source row **without** running
drop (W-N2 contract); (3) target owns its retained copies (memcpy'd at :488). The
`RemoveOutcome::Swapped` fixup in Phase 3 (re-resolving `&mut source` after the
hook, :511-513) **cannot observe a stale `source_row`** because deferred commands
the hook enqueued do **not** apply until the outermost drain (Q-A1) — which runs
*after* Phase 3 completes, so no command can mutate the source archetype between
Phase 2 and Phase 3. ✓

**Liveness:** `source`/`target` (`&mut`, :438-439) confined to Phase 1; Phase-3's
`&mut source` is re-resolved *after* the hook returns; at the `world_ptr` mint only
`source_ptr`/`target_ptr` survive. ✓ Hooks read the SOURCE dying row (§0). ✓

### §3.6 Despawn (`delete_entity`, ecs_master.rs:886-926)

```rust
self.enter_deferred_scope();                             // direct-API bracket (Q-A1)
let inland: EntityInland = { /* :889-897 read-by-value, drops entity_master borrow */ };
let removed_unit_index = InlandPoolId(inland.unit_index() as usize);

// Read flags + the component-id prefix into a STACK buffer (W1), entered ONLY
// when flags is non-empty (cold). No &mut Archetype held across the fire point.
let flags = unsafe { (*inland.archetype_ptr()).flags };
if !flags.is_empty() {
    let mut id_buf = [ComponentId(0); MAX_COMPONENTS];   // ~4 KB stack (W1), cold-only
    let n = {
        let arche = unsafe { &*inland.archetype_ptr() };  // SHARED, transient
        let ids = arche.component_ids.as_slice();
        id_buf[..ids.len()].copy_from_slice(ids);         // only [0..n) written (W1)
        ids.len()
        // <-- &Archetype drops here.
    };
    let world_ptr = NonNull::from(&mut *self);           // MINT: shared borrow dead
    // PRE-DROP (SAFETY-2): on_replace + on_remove for ALL, BEFORE remove_entity.
    if flags.contains(ArchetypeFlags::ON_REPLACE_HOOK) {
        for &cid in &id_buf[..n] { trigger_on_replace(world_ptr, cid, entity); }
    }
    if flags.contains(ArchetypeFlags::ON_REMOVE_HOOK) {
        for &cid in &id_buf[..n] { trigger_on_remove(world_ptr, cid, entity); }
    }
}

// Re-resolve &mut Archetype and proceed with the existing removal (:905-906).
let archetype: &mut Archetype = unsafe { &mut *inland.archetype_ptr() };
let outcome = archetype.remove_entity(removed_unit_index); // :906 — drops bytes
/* ... existing RemoveOutcome handling :908-925 ... */
self.exit_deferred_scope();
self.drain_deferred_hook_queue();                        // depth==0 here ⇒ drains
```

**Liveness:** the read-only `&Archetype` (for the id prefix) is confined to the
`let n = { ... }` block; the `&mut Archetype` (ecs_master.rs:905) is re-resolved
*after* the hook block; at the `world_ptr` mint only `inland.archetype_ptr()`
(`*mut`) survives. ✓ Hooks read the dying row PRE-`remove_entity`. ✓ The
`DespawnCommand::apply` path (despawn_command.rs:32-43) reaches this method at
`depth > 0`, so the end-of-method drain no-ops (C1). ✓

---

## §4 — Revised wave / step plan

The C2 migration rewrite is now a larger chunk (the two migration sites are
non-trivial phased restructures, not drop-ins); the bench gate moves to Wave 4
(C3). Each wave compiles independently and passes `cargo test --all-targets`.

### Wave 1 — `ArchetypeFlags` + field + layout tripwires (NO bench gate — C3)
1. **Step 1** — `ArchetypeFlags(u16)` type + bit consts + `contains`/`insert`/
   `is_empty` (new `core/component/hooks/archetype_flags.rs`). Unit tests.
2. **Step 2** — add `flags: ArchetypeFlags` to `Archetype` (§1-W2 placement, after
   `signature`, before `arena`); **measure** `size_of::<Archetype>()` and pin
   TRIPWIRE 1; **add** TRIPWIRE 2 (`ComponentLayout == 56`, currently doc-only at
   component_registry.rs:90); re-assert `offset_of!(columns) == 0` (:170);
   initialize `flags = empty()` in `Archetype::new` (:179) / `create_by_ids`
   (:203) / the slab path. **Acceptance: layout assertions compile.** No bench.

### Wave 2 — `ComponentHooks` table + `HAS_HOOKS` + install + flag compute
3. **Step 3** — `ComponentHooks`/`HookContext`/`HookFn` (§2.4) + `static HOOKS`
   (mirrors `LAYOUTS` :141) + `get_hooks`/`install_hooks::<C>` in
   component_registry.rs; O1 `Send + Sync` note. Re-assert `ComponentLayout == 56`.
4. **Step 4** — `Component::HAS_HOOKS` const + defaulted `register_hooks`
   (backward-compatible widening; every existing impl keeps `HAS_HOOKS = false`).
5. **Step 5** — `ArchetypeFlags::insert_from_hooks(cid)` wired into `create_by_ids`
   (archetype.rs:226-229) + `register_component_inplace` (:281-284) compute loops.
   Unit test: hooked archetype has the right bit; un-hooked has empty flags.

### Wave 3 — deferred queue + depth counter + read-only view (Q-A1 + Q-A2 + Q-A4)
6. **Step 6** — `EcsMaster::deferred_hook_queue: CommandQueue` + `hook_drain_depth:
   u32` fields (§2.3, after `arena`); `enter_deferred_scope`/`exit_deferred_scope`;
   `CommandQueue::apply_via_raw_twin(NonNull<EcsMaster>)` (raw-twin sibling, §2.2);
   `drain_deferred_hook_queue` with the depth gate + SAFETY-7 tripwire.
7. **Step 7** — `DeferredEcsMaster<'w>` reduced surface (§2.1) + `DeferredCommands`
   (`push` into `deferred_hook_queue`). Unit test: a `DeferredCommands` enqueue
   lands in `deferred_hook_queue` and applies on drain.

### Wave 4 — trigger fns + wire dispatch into the 6 sites + **the 0% bench gate**
8. **Step 8** — the `define_trigger!` macro (O2) → 5 `#[cold] #[inline(never)]`
   fns in `core/component/hooks/dispatch.rs`. Verify `#[cold]` demotes via
   `cargo asm`.
9. **Step 9** — wire spawn-deferred (§3.1) + spawn-direct (§3.2). Brackets +
   end-of-method drains on the direct paths.
10. **Step 10** — wire insert-in-place (§3.3, O3 ordering).
11. **Step 11** — wire the two migration sites: insert-migration (§3.4,
    `compute_added_mask` + repoint hoist) + remove-migration (§3.5, dual-presence
    Phase-1/2/3). **This is the big C2 chunk.**
12. **Step 12** — wire despawn (§3.6, W1 stack buffer).
13. **Step 13 (BENCH GATE — C3)** — bracket `system.apply` at schedule.rs:339 and
    :516 with `enter`/`exit` + the single outer drain; run the 0%-regression
    benches WITH real dispatch (populated-flags `u16` guarding a real `#[cold]`
    call, no-hook case). **MUST be within ±5% of pre-Phase-14a baseline.** Verify
    the not-taken arrangement via `cargo asm` on a real Wave-4 site.

### Wave 5 — derive macro attribute + runtime builder + release staleness (W3)
14. **Step 14** — `#[component(on_add = ..., ...)]` parsing + `HAS_HOOKS` +
    `register_hooks` codegen + install-into-`component_id()` (boyko_macros).
    trybuild tests for malformed attributes.
15. **Step 15** — `EcsMaster::register_component_hooks::<C>()` +
    `ComponentHooksBuilder` + the **release** staleness scan (Q-A5/W3).

### Wave 6 — tests + Miri + final re-gate
16. **Step 16** — full test suite (§8 of Round 1, plus the §3.5 dual-presence
    read-the-source test and the §3.4 read-the-target test).
17. **Step 17 (final re-gate)** — re-run the 0% benches with the full mechanism
    present but no hooks registered (still ±5%); separate informational bench of a
    hooked archetype's cold-path cost.

---

## §5 — Revised SAFETY invariants

- **SAFETY-1 (reentrancy / aliasing — paramount).** A hook's `DeferredEcsMaster`
  is minted from the same `&mut EcsMaster` the outermost apply holds. Soundness:
  (a) the view is **read-only into storage** + structural-withholding (§2.1 /
  Q-A2) — no `&mut`-into-pool method exists, so a hook **cannot construct** an
  aliasing `&mut`; (b) every fire site drops all `world`-derived `&mut Archetype` /
  `&mut ComponentPool` **before** minting `NonNull::from(&mut *world)` (§3,
  per-site liveness); (c) structural change is **deferred** (Q-A1) — a hook's
  `commands()` enqueues into `deferred_hook_queue`, applied only at the outermost
  boundary. There is no longer a "documented obligation the user can violate"
  (C2 closed).
- **SAFETY-2 (ordering — add before insert; replace+remove pre-drop).** Spawn
  fires all `on_add` then all `on_insert` (§3.1/§3.4); remove/despawn fire
  `on_replace` + `on_remove` **BEFORE** `drop_at` (migration_helpers.rs:505) /
  `remove_entity` (ecs_master.rs:906) so the hook reads live dying bytes.
  Insert-in-place fires `on_replace` (old value) then `on_insert` (new), not
  `on_add` (Q7). Enforced by call-site ordering.
- **SAFETY-3 (`ArchetypeFlags` correctness).** `flags` is the exact OR of every
  contained component's hook presence, computed at construction from `HOOKS[id]`
  (archetype.rs:226-229). Correctness requires register-before-use; a stale flag
  is now a **release panic** at `register_component_hooks` (Q-A5/W3), not a silent
  skip. The derive path is staleness-immune.
- **SAFETY-4 (apply-window-only firing).** Hooks fire ONLY inside the
  single-threaded outermost apply (schedule.rs:339, :516) or a direct-API
  `&mut EcsMaster` method — both under exclusive `&mut EcsMaster`. NEVER reachable
  from a parallel `&self` query path (Phase 9 workers never call structural ops).
- **SAFETY-5 (raw-twin drain non-aliasing).** `drain_deferred_hook_queue` runs
  `deferred_hook_queue.apply_via_raw_twin(world_ptr)` (§2.2). The raw twin
  (`CommandQueue::raw`, command_queue.rs:155-173) reads `bytes`/`cursor`/
  `panic_recovery` as `NonNull` via `&raw mut` **without** an intermediate `&mut`
  on those fields, and `world_ptr` accesses EcsMaster's *other* fields — so the
  queue field and the world pointer do not alias. Survivors-on-panic preserved via
  the same single `catch_unwind` + `handle_panic_recovery(0)` re-absorb
  (:560-566). The `mem::take`-into-local discard hazard (W5) is gone.
- **SAFETY-6 (panic during apply).** A hook panic during a *command body* unwinds
  through the existing `consume_and_drop_glue` W3' cursor-advance
  (command.rs:149), `CursorSync` guard (command_queue.rs:291-319), and the single
  per-system `catch_unwind` (:244) — survivors recover, the panic propagates. A
  deferred-command panic during the *drain* unwinds through the drain's own single
  `catch_unwind` — the two are **sequential, never nested** (C1 proof, §1-C1). No
  new `catch_unwind` levels are introduced.
- **SAFETY-7 (`IN_SYSTEM_RUN == false` at fire sites).** Hooks + their deferred
  commands run with `IN_SYSTEM_RUN == false` — **verified**: `InSystemRunGuard`
  wraps only `run_unsafe` (tls.rs:152-159; schedule.rs:605 created, :623 dropped
  *before* completion publish and before `apply`); the inline-exclusive `apply`
  (schedule.rs:516) runs outside any guard. So a hook's deferred arena allocation
  does not trip ALLOC1 (`debug_assert!(!IN_SYSTEM_RUN)`, tls.rs:37). A
  `debug_assert!(!is_in_system_run())` tripwire at the top of
  `drain_deferred_hook_queue` (§2.2) catches a future refactor that moved `apply`
  inside the guard (W4).

Every `unsafe` block in the implementation carries a `// SAFETY:` comment citing
the relevant invariant above (CLAUDE.md #8).

---

## §6 — Revised risk register

| Risk | Severity | Round 2 resolution |
|---|---|---|
| **Reentrancy UB** (hook does structural change inline) | Critical | Read-only structural-withholding view (Q-A2) + deferred queue drained at the single outermost boundary with a depth counter (Q-A1). No inline structural change reachable. |
| **Two-level panic recovery / stale replay** (C1) | Critical | Single drain owner + depth gate ⇒ the per-system `catch_unwind` and the drain `catch_unwind` are sequential, never nested (§1-C1 proof). No double-apply, no cross-frame stale replay. |
| **Migration-site aliasing** (C2) | Critical | All 6 sites phased: drop all `world`-derived `&mut` before minting `world_ptr` (§3); per-site liveness argued. `get_component_mut` dropped (Q-A2). |
| **0% gate proves nothing** (C3) | High | Bench gate moved to Wave 4 with real dispatch (populated-flags `u16` guarding a real `#[cold]` call); `cargo asm` verification on a real site. |
| **Per-despawn heap alloc** (W1) | High | Stack `[ComponentId; MAX_COMPONENTS]` buffer, cold-only, prefix-write (§3.6 / §1-W1). |
| **Wrong layout justification** (W2) | Medium | Corrected to +8 B (signature is `#[repr(align(32))]`); two hard `const _` tripwires (size + `ComponentLayout == 56`). |
| **Staleness silent-skip** (W3) | Medium→High | Release panic at `register_component_hooks` (Q-A5); derive path immune. |
| **ALLOC1 interaction unstated** (W4) | Medium | SAFETY-7 + drain tripwire; verified guard does not wrap `apply`. |
| **`mem::take` drain discards survivors on panic** (W5) | High | Replaced by raw-twin (Q-A4); survivors re-absorb into the same `bytes`. |
| **Re-entrant hook chain non-termination** | Low | User-error (acyclic graph assumption), same class as infinite recursion; each drain turn is a sound apply. |
| **Macro attribute parse errors** | Low | trybuild tests for malformed `#[component(...)]`. |
| **`Added<T>` overlap** (DOTS lesson) | Low (scope) | Documented: prefer Phase 10 `Added<T>` for query-side reactions; hooks for synchronous structural-op-time effects. |

---

## §7 — Residual open questions for the architecture-critic (Round 2)

Three areas the architect flags for the critic's scrutiny (everything else is
believed closed):

1. **§3.5 dual-presence window soundness.** The momentary state where the entity
   is in both the source row (`C` live) and target row (`C` absent) with
   `EntityInland` at SOURCE. The argument is: `drop_at` drops `C` once (Phase 3),
   `move_out_entity` swap-removes without drop, target owns retained copies, and
   deferred commands cannot mutate source between Phase 2 and Phase 3 (drain is at
   the outermost boundary). The critic should confirm no reader (hook
   `get_component` against a *third* entity that was swap-moved) can observe an
   inconsistent `EntityInland`/row pairing during Phase 2.
2. **§1-C1 depth-bracket minimal correctness.** The proof rests on `system.apply`
   (schedule.rs:339, :516) fully returning (its `catch_unwind` off the stack)
   *before* the drain runs at depth 0. The critic should confirm there is no path
   where a command's `apply` re-enters another `system.apply` (there is not —
   APP4/CQ7 forbid re-entry into `run_system_once`), and that the direct-API
   `enter`/`exit` brackets cannot nest with a command-queue apply in a way that
   leaves depth > 0 when it should be 0.
3. **§3.4 repoint-hoist asymmetry.** Hoisting the `EntityInland` repoint
   (migration_helpers.rs:402-403) *into* the Phase-1 block (so insert hooks read
   the NEW target row) while remove hooks read the SOURCE row (no hoist, §3.5) is
   the deliberate Bevy-parity asymmetry. The critic should confirm the hoist does
   not break the `RemoveOutcome::Swapped` fixup in the insert path
   (migration_helpers.rs:392-394 touches a *different* entity's inland) and that
   reading the target row is the intended add/insert semantic.

---

## Changelog (Round 1 → Round 2)

| Finding | Round 1 position | Round 2 disposition | Section |
|---|---|---|---|
| **C1** drain double-driven + unproven nested panic | "both" drive points (§5.4) | ONE owner at outermost boundary + depth counter; step-by-step single-`catch_unwind` proof | §0 Q-A1, §1-C1, §2.2/§2.3 |
| **C2** `get_component_mut` footgun + migration `&mut` across fire | documented obligation; drop only at 2 sites | DROP `get_component_mut` (read-only view); phased restructure of all 6 sites | §0 Q-A2/Q-A3, §1-C2, §2.1, §3 |
| **C3** Wave 1 gate is DCE'd empty block | go/no-go bench in Wave 1 | bench gate → Wave 4 (real dispatch); Wave 1 = field + asserts; `cargo asm` | §1-C3, §4 |
| **W1** per-despawn `to_vec()` alloc | cold `to_vec()`, follow-up micro-opt | stack `[ComponentId; MAX_COMPONENTS]` prefix buffer, cold-only | §1-W1, §3.6 |
| **W2** "zero growth" claim false | "lands in padding" | corrected to +8 B; TRIPWIRE 1 (size) + TRIPWIRE 2 (`ComponentLayout == 56`, added) | §1-W2, §2.4 |
| **W3** staleness silent-skip in release | debug-assert only | release panic at `register_component_hooks`; derive immune | §0 Q-A5, §1-W3, §5 SAFETY-3 |
| **W4** ALLOC1 interaction unstated | (missing) | SAFETY-7 + drain `debug_assert!(!is_in_system_run())`; verified guard scope | §1-W4, §5 SAFETY-7 |
| **W5** `mem::take` drain discards survivors | take-restore dance | raw-twin (`CommandQueue::raw`); survivors re-absorb same `bytes` | §0 Q-A4, §1-W5, §5 SAFETY-5 |
| **O1** `HOOKS` Send/Sync note | (missing) | one-line note: fn-pointer-only ⇒ auto `Send + Sync` | §1-O1, §2.4 |
| **O2** 5 near-identical trigger fns | hand-written ×5 | `define_trigger!` declarative macro + `cargo asm` `#[cold]` check | §1-O2 |
| **O3** insert-replace closure placement | "settled" | covered by C2 restructure (drop closure `&mut` before fire) | §1-O3, §3.3 |
| **OQ1** drain ownership | "both" | outermost boundary only | §0 Q-A1 |
| **OQ2** `get_component_mut` in 14a? | exposed | dropped → 14b | §0 Q-A2 |
| **OQ3** re-entrant drain visibility | mem::take (invisible) | raw-twin (visible) | §0 Q-A4 |
| **OQ4** staleness in release | silent-skip | release panic | §0 Q-A5 |
| **OQ5** what does remove-hook observe? | unstated | SOURCE (dying) row; insert reads TARGET (asymmetry stated) | §0, §3.4, §3.5 |

---

## Readiness statement

**Ready for architecture-critic Round 2.**

All 3 CRITICAL findings (C1 single-owner drain + panic proof; C2 read-only view +
6-site phased restructure; C3 bench gate → Wave 4), all 5 important findings
(W1-W5), and all 3 optional findings (O1-O3) are resolved with concrete revised
designs grounded in source. The 5 open questions are answered decisively (§0).

The three areas wanting the **most scrutiny** (§7): (1) the **§3.5 dual-presence
window** soundness during remove-migration Phase 2; (2) the **§1-C1
depth-bracket** minimal-correctness (that `system.apply` always fully returns
before the depth-0 drain, with no re-entry path); (3) the **§3.4 repoint-hoist
asymmetry** (insert hooks read target, remove hooks read source — Bevy parity).

---

## §8 — Round 3 Patches (supersede the referenced passages)

> These patches address `PHASE-14-CRITIC-ROUND-2.md` (verdict **REVISE**:
> NEW-1 CRITICAL, NEW-2 + W5-statement HIGH, NEW-3 + W2-literal SHOULD). The
> critic CONFIRMED C1/C2/C3 RESOLVED in design; these patches close the *new*
> issues exposed by opening the real source at the direct-API + migration sites.
> Where a patch conflicts with an earlier passage, **the patch wins**.

### P1 (NEW-1, CRITICAL) — `hook_drain_depth` bracket must be RAII (exception- and early-return-safe)

**Problem.** §2.2's plain `enter_deferred_scope()`/`exit_deferred_scope()` calls
and §3.2/§3.6's linear `enter … exit` sketch **leak the depth increment** on the
early-`return Err` paths in `create_entity` (ecs_master.rs:582-593,
`ArchetypeRejectedEntity`) and `create_entity_at` (:642 `ArchetypeNotFound`,
:689-691 `!pushed`), and on any panic. A leaked increment leaves
`hook_drain_depth >= 1` forever → every subsequent `drain_deferred_hook_queue`
sees `depth != 0` and **silently stops draining for the rest of the process** —
deferred hook commands accumulate and never apply. This is reachable on ordinary
error paths, no panic required.

**Fix.** `enter_deferred_scope` returns a RAII guard whose `Drop` decrements the
depth — mirroring the codebase's existing `CursorSync` (command_queue.rs:291-319)
and `InSystemRunGuard` (tls.rs:142-166) discipline. Every exit path (`Ok`, `Err`,
unwind) decrements correctly. The drain is invoked **explicitly on the success
path only**, after the guard is dropped, at depth 0:

```rust
/// RAII depth bracket. Drop decrements `hook_drain_depth` on EVERY exit path
/// (Ok / Err / panic), so the depth can never leak (NEW-1). Holds a raw
/// NonNull (not &mut) so it does not freeze `world` for the bracketed region.
pub(crate) struct DeferredScopeGuard {
    world: NonNull<EcsMaster>,
}
impl Drop for DeferredScopeGuard {
    #[inline]
    fn drop(&mut self) {
        // SAFETY: minted from the &mut EcsMaster that owns this guard; the guard
        //   never outlives that borrow (it is a stack local of the bracketed fn).
        unsafe { (*self.world.as_ptr()).hook_drain_depth -= 1; }
    }
}
impl EcsMaster {
    #[inline]
    fn enter_deferred_scope(&mut self) -> DeferredScopeGuard {
        self.hook_drain_depth += 1;
        DeferredScopeGuard { world: NonNull::from(&mut *self) }
    }
}
```

Revised direct-API usage (the §3.2 / §3.6 bracket pattern):

```rust
fn create_entity(&mut self, ...) -> EcsResult<Entity> {
    let scope = self.enter_deferred_scope();      // depth += 1; RAII
    // ── fallible setup: every `return Err(...)` here drops `scope`
    //    (depth -= 1) and strands NOTHING, because all Err returns precede
    //    the hook-fire point (no command has been enqueued yet). ──
    // ... create entity, register_entity_with_ptr ...
    // ... fire on_add-all then on_insert-all (enqueue into deferred_hook_queue) ...
    drop(scope);                                   // depth back to 0 (direct API)
    self.drain_deferred_hook_queue();              // runs (depth == 0)
    Ok(entity)
}
```

**Why explicit-drain-on-success is sufficient.** In `create_entity` /
`create_entity_at`, every `return Err` occurs **before** the hook-fire point
(verified: the push/`register_entity_with_ptr` succeeds first, hooks fire last),
so no deferred command is ever enqueued on an `Err` path — there is nothing to
drain. On the panic path, the guard's `Drop` restores depth (no leak), and we
deliberately do **not** drain during unwind (running deferred user code mid-panic
is wrong); any commands a pre-panic hook enqueued persist in `deferred_hook_queue`
and are picked up by the next outermost drain — consistent with the existing
command panic-recovery survivor semantics (command_queue.rs:560-566). The
command path is unchanged: `delete_entity` called from `DespawnCommand::apply`
runs at `depth >= 1` (the per-system `CommandQueue::apply` bracket), so its
end-of-method `drain_deferred_hook_queue` sees `depth != 0` and returns early.

> **Invariant for the developer:** never place a fallible step that can
> `return Err` *after* a hook-fire point inside a bracketed body. If a future
> change requires one, the enqueued commands must be drained (or explicitly
> retained) before the early return. Add a `debug_assert!` documenting this.

**The RAII guard applies at ALL FIVE bracket sites** (not only the direct-API
methods the worked example shows): (1) `create_entity`, (2) `create_entity_at`,
(3) `delete_entity`, and the two **schedule apply sites** — (4) `apply_window_drain`'s
`system.apply(world)` (schedule.rs:339) and (5) the inline-exclusive path's
`system.apply(world_ref)` (schedule.rs:516). The §3.2 / §3.6 sketches that still
show the bare `enter_deferred_scope()` / `exit_deferred_scope()` pair are
superseded by P1 — every site uses the guard. The two schedule sites are the
**highest panic risk** (they run arbitrary user `CommandQueue::apply`, and there
is **no schedule-level `catch_unwind`** around schedule.rs:339 — the only catch
is *inside* `CommandQueue::apply` at command_queue.rs:244): the guard MUST be held
across the whole `system.apply(world)` call so a command panic propagating up
through the un-caught :339 site still decrements the depth via the guard's `Drop`
during unwind, then `drain_deferred_hook_queue` runs once after `system.apply`
returns at depth 0. Without the RAII guard, that panic path would leak the depth
permanently (the NEW-1 hazard, at the site most likely to panic).

### P2 (W5-statement, HIGH) — `drain_deferred_hook_queue` must not hold `&mut self` and `&mut self.deferred_hook_queue` simultaneously

**Problem.** §2.2 line 545 — `unsafe { self.deferred_hook_queue.apply_via_raw_twin(NonNull::from(&mut *self)); }`
— takes the method receiver `&mut self.deferred_hook_queue` AND `&mut *self`
(for the `NonNull`) in one statement. That is the exact field-alias the raw-twin
sibling exists to avoid; it will not compile / is UB if forced.

**Correction (Round 3 critic, P2 was over-corrected).** An earlier draft of
this patch proposed calling the raw twin's walk directly and deleting the
`apply_via_raw_twin` sibling. That was wrong on three counts the critic verified
against real source, and is hereby reverted:

1. `RawCommandQueue::apply(NonNull<EcsMaster>)` **does not exist** — the twin's
   only walk is `apply_or_drop_queued_no_catch(&mut self, Option<NonNull<EcsMaster>>)`
   (command_queue.rs:341), which is **catch-FREE by contract** (its doc:
   *"the outer caller MUST invoke `handle_panic_recovery` before `resume_unwind`"*).
2. `RawCommandQueue` and `CommandQueue::raw()` are **private** to
   `command_queue.rs` (no `pub`/`pub(crate)`); `drain_deferred_hook_queue` lives
   in `ecs_master.rs` and cannot name them.
3. Calling the catch-free walk directly would **discard the single
   `catch_unwind` + `handle_panic_recovery(0)` survivor re-absorb** that
   W5/SAFETY-5 require and would falsify SAFETY-6 ("the drain has its own single
   `catch_unwind`"). The catch lives in `CommandQueue::apply`
   (command_queue.rs:244-250), **not** in the twin.

**Fix.** Keep the `apply_via_raw_twin` sibling (the original §2.2 instinct was
right). It is a `pub(crate)` **associated function** on `CommandQueue`, defined
*in* `command_queue.rs` so `raw()` / `RawCommandQueue` stay private. It mirrors
`CommandQueue::apply` (command_queue.rs:203-251) — the `raw()` mint + the single
`catch_unwind` + `handle_panic_recovery(0)` — but takes the world as
`NonNull<EcsMaster>` (not `&mut`) and derives the queue from it, because the
deferred queue is a **field** of `world`:

```rust
// In command_queue.rs (keeps raw()/RawCommandQueue private). Associated fn, NOT
// a &mut self method: the queue is reached through `world`, so there is never a
// `&mut self`(queue) receiver to alias `&mut *world`.
impl CommandQueue {
    /// Drain `world.deferred_hook_queue` with full `apply` semantics (single
    /// catch_unwind + handle_panic_recovery(0) survivor re-absorb), taking the
    /// world as NonNull because the queue is a field of it.
    ///
    /// # Safety
    /// - `world` is a valid, exclusively-borrowed `EcsMaster` (caller holds
    ///   `&mut EcsMaster` and minted this `NonNull` from it; SAFETY-4 window).
    /// - All queue access (bytes/cursor/panic_recovery) is threaded through the
    ///   raw twin's `NonNull` (CursorSync discipline, command_queue.rs:291-319);
    ///   a transient `&mut *world` is formed ONLY per `cmd.apply`, never while a
    ///   `&mut`-into-the-queue is live. The bytes `Vec`'s heap buffer is a
    ///   separate allocation from `EcsMaster`, so the byte walk never aliases
    ///   `&mut *world`; the in-`EcsMaster` cursor/recovery writes are sequenced
    ///   through raw pointers, never simultaneous with `&mut *world` (SAFETY-5).
    pub(crate) unsafe fn apply_via_raw_twin(world: NonNull<EcsMaster>) {
        // SAFETY: transient &mut to mint the raw twin; dropped immediately.
        let mut twin = unsafe { (*world.as_ptr()).deferred_hook_queue.raw() };
        // Single catch — identical to apply()'s :244-250.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // SAFETY: catch-free walk; forms transient &mut *world per command.
            unsafe { twin.apply_or_drop_queued_no_catch(Some(world)); }
        }));
        if let Err(payload) = result {
            // SAFETY: re-absorb survivors into the SAME bytes (start==0), then
            //   re-raise — mirrors apply() (:248) + handle_panic_recovery (:560-566).
            unsafe { twin.handle_panic_recovery(0); }
            std::panic::resume_unwind(payload);
        }
    }
}
```

Revised drain loop (`ecs_master.rs`) — no `&mut`-into-queue is ever held across
the apply, because `apply_via_raw_twin` takes only `world: NonNull` and derives
the queue internally:

```rust
fn drain_deferred_hook_queue(&mut self) {
    if self.hook_drain_depth != 0 { return; }                       // Q-A1 gate
    debug_assert!(!boyko_threadpool::is_in_system_run());           // SAFETY-7 (W4)
    let world_ptr: NonNull<EcsMaster> = NonNull::from(&mut *self);
    loop {
        // Transient shared borrow only for the emptiness test; dropped at `;`.
        // SAFETY: world_ptr valid + exclusive (&mut self at the call site).
        if unsafe { (*world_ptr.as_ptr()).deferred_hook_queue.is_empty() } {
            break;
        }
        // Re-entrant appends pushed during this call land in the SAME bytes and
        // are seen by the next `is_empty()` (the twin re-reads `bytes.len()`).
        // SAFETY (SAFETY-5/-6): full catch+recovery semantics; no &mut-into-queue
        //   held across the per-command `&mut *world` (proven in the fn's SAFETY).
        unsafe { CommandQueue::apply_via_raw_twin(world_ptr); }
    }
}
```

This restores the §2.2 sibling (delete the "not a method / raw twin already
provides the walk" claim), keeps `raw()`/`RawCommandQueue` private, preserves the
single-`catch_unwind` survivor re-absorb (W5/SAFETY-5/-6), and — because the
sibling takes `world: NonNull` and never holds a `&mut self`(queue) receiver —
fully eliminates the field-alias that W5-statement flagged.

### P3 (NEW-2, HIGH) — `on_insert` must fire for the BUNDLE set, not all `target_ids`

**Problem.** §3.4 line 744 — `for &cid in target_ids { trigger_on_insert(...) }`
— fires `on_insert` for **every** component in the target archetype. But
`target = source ∪ bundle` (migration_helpers.rs:205-247 retains `source ∩ target`,
:261-277 adds the bundle, :313-354 merges with bundle-wins). So `target_ids`
includes **retained-not-in-bundle** components (`source \ bundle`) that the entity
already had and that are merely carried over — firing `on_insert` for them is a
spurious callback and diverges from Bevy (insert fires `on_insert` only for the
inserted bundle's components) and from the Round 1 Q7 decision.

**Fix.** Capture the bundle's component-id set during the existing Step-2 bundle
iteration (migration_helpers.rs:261-277, which runs **before** `move_out_entity`
at :387 — satisfying the critic's "pin ordering before `move_out_entity`"), and
fire `on_insert` over that set:

```rust
// In PHASE 1, during the existing `bundle.for_each_component_bytes(|id, bytes| …)`
// loop (migration_helpers.rs:261-277), also record the bundle's ids:
let mut bundle_ids = [ComponentId(0); MAX_BUNDLE_ARITY];
let mut bundle_id_count = 0usize;
bundle.for_each_component_bytes(|id, bytes| {
    bundle_ids[bundle_id_count] = id;       // capture I (the inserted set)
    bundle_id_count += 1;
    /* ...existing bundle_slots write... */
});
// `added_mask` is unchanged and correct: T\S == I\S (target = source ∪ bundle),
// so "target ids not in source" == "bundle ids not in source" == newly-added.

// PHASE 2 — fire hooks (target_ptr live, source/target &mut dropped):
let bundle_id_set = &bundle_ids[..bundle_id_count];   // == I
if flags.contains(ArchetypeFlags::ON_ADD_HOOK) {
    for &cid in bundle_id_set {
        if added_mask.contains(cid) { trigger_on_add(world_ptr, cid, entity); } // I\S
    }
}
if flags.contains(ArchetypeFlags::ON_INSERT_HOOK) {
    for &cid in bundle_id_set { trigger_on_insert(world_ptr, cid, entity); }    // I
}
```

Firing matrix for migration-insert (entity {S} + insert bundle {I}, S≠T=S∪I):
`on_add` → `I \ S` (newly present); `on_insert` → `I` (inserted set);
retained-not-in-bundle `S \ I` → **nothing**. This matches Bevy and Round 1 Q7.

> **Residual (flag for the critic / 14b):** Bevy also fires `on_replace` for
> `I ∩ S` (bundle components that overwrite an existing value) on the insert
> path. Round 1's §4.3 table scoped migration-insert to `on_add + on_insert`
> only (no `on_replace`), and Round 2 preserves that. If full Bevy parity is
> wanted, add `on_replace` over `bundle_id_set ∩ source` (pre-overwrite) — but
> that is a deliberate scope decision, not a bug, so it is recorded here rather
> than silently expanded. The in-place replace path (§3.3) already fires
> `on_replace` per Q7; only the *migration*-overlap case is deferred.

### P4 (NEW-3, SHOULD) — bench the no-hook path of the functions that grew a 4 KB frame

The C3 gate (§4 Wave 4 Step 13 / Wave 6 Step 17) currently names only
`system.apply`-shaped benches. But §3.4/§3.6 add a `[ComponentId; MAX_COMPONENTS]`
(~4 KB; `MAX_COMPONENTS = 512`, `ComponentId = usize`) **stack local** to
`delete_entity` (ecs_master.rs:886) and the spawn-direct path — and a larger stack
frame can perturb no-hook-path codegen (prologue `sub rsp`, spill choices) even
when the cold branch is never taken. The C3 branch-bench would not catch this.

**Add to the Step 13 + Step 17 gate:** dedicated `delete_entity` (despawn-single)
and `create_entity` (spawn-single) **no-hook** benches, compared to the
pre-Phase-14a baseline. **Named mitigation if a frame regression shows:** hoist
the id buffer out of the per-call frame into an `EcsMaster`-resident scratch
(precedent: the `migration_scratch` reference at migration_helpers.rs:38) or into
a `#[cold] #[inline(never)]` helper fn that owns the buffer, so the hot no-hook
caller's frame is unchanged.

### P5 (W2-literal, LOW) — TRIPWIRE 1 must pin a measured literal, not a tautology

§2.4 TRIPWIRE 1 as written (`assert!(size_of::<Archetype>() == 0 + size_of::<Archetype>())`)
is a tautology and guards nothing. **Wave 1 Step 1** must: (1) print
`size_of::<Archetype>()` in a one-off test on the target, (2) replace the
placeholder with the concrete literal (`const _: () = assert!(size_of::<Archetype>() == <N>);`),
so the assertion actually pins the layout. The pre-Phase-14a size + 8 B (the W2
correction) is the expected value; confirm by measurement, do not assume.

### Round 3 changelog

| Critic R2 item | Severity | Disposition | Patch |
|---|---|---|---|
| NEW-1 depth bracket leaks on early-`return Err` / panic | CRITICAL | RAII `DeferredScopeGuard` + explicit success-path drain | P1 |
| W5-statement field-alias in §2.2 drain | HIGH | re-derive twin, drop `&mut` field before apply | P2 |
| NEW-2 `on_insert` over-fires retained components | HIGH | fire over captured `bundle_ids` (I), not `target_ids` (T) | P3 |
| NEW-3 4 KB frame perturbs no-hook path | SHOULD | add `delete_entity`/`create_entity` no-hook benches + mitigation | P4 |
| W2 TRIPWIRE 1 tautology | LOW | measure + pin literal in Wave 1 Step 1 | P5 |

C1/C2/C3 + W1/W3/W4 + O1/O2/O3 were CONFIRMED RESOLVED by critic Round 2 and are
unchanged. §3.5 dual-presence, §1-C1 depth bracket, §3.4 repoint-hoist (the 3
residuals) were verified sound by critic Round 2 (NEW-4/5/6 RESOLVED).
