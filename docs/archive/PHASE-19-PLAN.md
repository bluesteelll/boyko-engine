# Phase 19 — Hierarchies / Parent-Child (CORE) — Architect Plan

Implementation-ready plan. Research: `docs/PHASE-19-RESEARCH.md`. Builds on the 14a/14b hook substrate.
**Goal:** `ChildOf` (FK on child) + `Children` (reverse collection on parent) kept bidirectionally
consistent by component hooks, + recursive despawn cascade + `EntityCommands` ergonomics. The binding
perf requirement is **0% cost for non-hierarchy entities** (the existing `ArchetypeFlags` u16 gate).
All hierarchy mutation is on the cold structural-op path; the hot iteration path is untouched.

## 1. Final decisions
1. **`Children` mutation → option (a): bespoke deferred commands.** `ChildOf` hooks enqueue
   `LinkChildCommand{parent,child}` / `UnlinkChildCommand{parent,child}` via
   `DeferredEcsMaster::commands()`; at apply time under `&mut EcsMaster` they mutate the parent's
   `Children` via Phase-14b `get_component_mut::<Children>` (get-or-insert) + Vec push/swap-remove.
   Reuses the deferred queue + 14b verbatim — **no ECS-core view change, no new Tree-Borrows surface**.
   Trade-off: `Children` consistent only after the drain (same-frame, at the apply window — accepted).
   Rejected (b) `children_mut` on the view (re-opens the `&mut`-into-storage hole Q-A2 closed).
2. **Invariant carrier → `ComponentHooks` (type-level fn-ptr), not observers.** `ChildOf`/`Children` are
   engine-defined → **hand-impl `Component`** with `HAS_HOOKS=true` + `register_hooks`; slots install via
   the existing `install_hooks::<C>` path on first `component_id()`. Observers stay for user reactions.
3. **`Children` representation → `Vec<Entity>`** (Bevy-parity, dense, 24 B header, allocates on first
   child = cold). Inline-small = deferrable. Removal uses **`swap_remove`** (find-index + `Vec::swap_remove`,
   O(1) removal, no order contract in CORE — matches boyko's swap-remove-everywhere principle).
4. **Cascade → default-recursive despawn** + opt-out `despawn_without_children`. Hierarchies are new (no
   non-cascade behavior to break); default-recursive = Bevy 0.16 `LINKED_SPAWN`. Cascade hangs off
   **`Children::on_replace` firing in `delete_entity`** (boyko has no on_despawn kind; the per-component
   Replace hook `delete_entity` already fires at ecs_master.rs:1087-1098 is the equivalent, reading the
   CURRENT collection — #17883-safe).
5. **Ordering + atomicity → binding + tested** (see §5).
6. **Concrete `ChildOf`/`Children` only** — no generic `Relationship` trait (YAGNI; note as future).

## 2. Types
New module `crates/boyko_ecs/src/ecs/core/hierarchy/` (`mod.rs` components+hooks, `commands.rs` Link/Unlink).
```rust
#[repr(transparent)] pub struct ChildOf(pub Entity);     // 8 B, FK on child, source of truth
#[repr(transparent)] pub struct Children(Vec<Entity>);   // 24 B, reverse collection on parent
// Children: pub as_slice/len/is_empty/contains; pub(crate) push / swap_remove_entity (only Link/Unlink mutate)
pub(crate) struct LinkChildCommand   { parent: Entity, child: Entity }  // 16 B repr(C), unsafe impl Send+Sync (POD)
pub(crate) struct UnlinkChildCommand { parent: Entity, child: Entity }
```
**EntityCommands/Commands additions** (thin wrappers over insert/remove/despawn): `add_child`,
`add_children(&[Entity])`, `set_parent`, `remove_parent`, `remove_children(&[Entity])`, `clear_children`,
`despawn_without_children`; `Commands::add_child(parent, child)`. The relationship is driven ENTIRELY by
`ChildOf` insertion (`add_child(p,c)` == `child.insert(ChildOf(p))`); user code never writes `Children`.

## 3. Hook wiring (all follow OBS-FIRE-LOOP / F2 discipline: read view, copy out scalars, never hold a
`world`-derived `&` across `commands()`/enqueue)

**`ChildOf::on_insert` (LINK)** — fires after the NEW ChildOf is written (migrate add / reparent insert):
```
let child = ctx.entity; let parent = view.get_component::<ChildOf>(child)?.0;   // copy out; &drops
if parent == child { debug_warn; view.commands().entity(child).remove::<ChildOf>(); return; }  // self-ref
if !view.has_parent(parent) { debug_warn; view.commands().entity(child).remove::<ChildOf>(); return; } // dangling
view.commands().add(LinkChildCommand{parent, child});
```
**`ChildOf::on_replace` (UNLINK)** — fires before the OLD ChildOf leaves (reparent overwrite / remove /
despawn cascade), reading the dying value:
```
let child = ctx.entity; let old = view.get_component::<ChildOf>(child)?.0;  // OLD value; &drops
view.commands().add(UnlinkChildCommand{parent: old, child});   // harmless no-op if not present (apply guard)
```
**`Children::on_replace` (CASCADE)** — fires before Children leaves the parent; in `delete_entity` this
fires for every component on the dying entity BEFORE `remove_entity` (the #20106-correct order). Reads
the CURRENT collection. The ONE loop needing per-turn re-derivation:
```
let parent = ctx.entity;
let children = view.get_component::<Children>(parent)?;        // world-derived &
let n = children.len();
if n <= CASCADE_FANOUT_INLINE {                                // fast path: copy to stack, & drops, then enqueue
    let mut buf=[Entity::PLACEHOLDER; CASCADE_FANOUT_INLINE]; buf[..n].copy_from_slice(children.as_slice());
    // &Children drops HERE, before commands() minted (F2-sound)
    let mut cmds = view.commands(); for &c in &buf[..n] { cmds.entity(c).despawn(); }
} else {                                                       // cold wide path: re-derive per turn
    let mut i=0; loop { let next = { let ch=view.get_component::<Children>(parent)?; if i>=ch.len(){break} ch.as_slice()[i] };
                        view.commands().entity(next).despawn(); i+=1; }
}
```
F2-sound: the only `world`-derived borrow (`&Children`) drops before `commands()` mints its
`NonNull<EcsMaster>` — mirrors the audited `fire_*_observers` loop. Recursion: each `despawn` pushes a
`DespawnCommand` to the world deferred queue; they drain at the single outermost
`drain_deferred_hook_queue` (each inner `delete_entity` at depth≥1 no-ops its own drain); grandchildren
enqueued mid-drain are absorbed by the `while !is_empty()` loop. No manual stack.

**`DeferredEcsMaster::has_parent(&self, Entity)->bool`** — the ONLY view addition: a read-only existence
check delegating to `EcsMaster::has_entity` (no `&mut`-into-storage, no new TB surface).

## 4. Apply-time command bodies (under `&mut EcsMaster` during the depth-guarded drain)
**`LinkChildCommand::apply`:** `if !world.has_entity(parent){return}`; `match get_component_mut::<Children>(parent)
{ Some(mut c)=>c.push(child), None=> world.insert_one(parent, Children::with_one(child)) }`. First-child
insert is a structural op done INLINE under the held `&mut` (reuses InsertCommand machinery); inserting
`Children` fires `on_add`/`on_insert` (which `Children` does NOT register) — NOT `on_replace` — so no
spurious cascade.
**`UnlinkChildCommand::apply`:** `let Some(mut c)=get_component_mut::<Children>(parent) else {return}; c.swap_remove_entity(child);
if c.is_empty(){ drop(c); world.remove_one::<Children>(parent) }`. Removing empty Children fires
`Children::on_replace` on a LIVE parent → cascade loop sees 0 children → no spurious despawn (the
`len()==0` early-out covers it).
**`ClearChildrenCommand::apply`** (backs clear_children/remove_children): read CURRENT Children at apply
time, per child `remove_one::<ChildOf>(c)` (each fires ChildOf::on_replace → UnlinkChild) — #17883-safe.
**Recursive `despawn`:** no new command — existing `DespawnCommand`→`delete_entity` fires `Children::on_replace`.
`despawn_without_children` → a `CASCADE_SUPPRESS` thread-local (mirrors HOOK_DRAIN_DEPTH) read at the top
of `Children::on_replace`; set for the single `delete_entity` call.

## 5. Ordering + atomicity invariants (binding + tested)
- **(i) #20106 read-before-remove:** cascade reads Children BEFORE `remove_entity` (boyko pre-remove
  firing at ecs_master.rs:1087-1098/1138-1152 is correct — made binding + tested).
- **(ii) Reparent atomicity:** reparent = `child.insert(ChildOf(B))` → on_replace enqueues UnlinkChild(A)
  THEN on_insert enqueues LinkChild(B); FIFO drain applies unlink-then-link; no user code between →
  post-drain child in exactly one list (B's). Test.
- **(iii) self-ref + dangling guards:** rejected reactively (component removed, no Children mutation);
  never corrupt a collection. Test both.

## 6. 0%-when-unused
Non-hierarchy entity: archetype `ArchetypeFlags` raises no ChildOf/Children hook bit (OR-computed from
the cold HOOKS table, only for present components) → same `if !flags.is_empty()`/`contains(ON_*_ANY)`
gate (one u16 load + test/jz, not-taken) as 14a/14b. Iteration never touches hooks. A hierarchy-free
program never mints ChildOf/Children component_id → HOOKS slots stay None. Validation: the
`phase14a_hooks_gate` bench stays flat vs baseline (binding 0%-gate bench).

## 7. SAFETY inventory
Expected **zero new `unsafe` blocks** in hierarchy code: hook bodies use safe `view.get_component`/
`commands()` + scalar copies + safe `copy_from_slice`; command applies use safe `has_entity`/
`get_component_mut` (returns `Mut`) + safe Vec ops + crate-internal `insert_one`/`remove_one` (which
reuse the ALREADY-audited InsertCommand/migrate_entity_* unsafe — no NEW unsafe authored). `CASCADE_SUPPRESS`
= safe `Cell<bool>`. The `unsafe fn` hook items have all-safe bodies (the keyword is the dispatch
contract). If `insert_one`/`remove_one` extraction forces an `unsafe`, it carries a SAFETY citing `&mut
EcsMaster` exclusivity + the InsertCommand precedent (flagged for the critic — §open).

## 8. Developer wave breakdown (each wave compiles)
- **W1 types + registration:** `hierarchy/mod.rs` ChildOf/Children + accessors + hand-impl Component
  (HAS_HOOKS, register_hooks STUBS) + prelude re-exports + `Entity::PLACEHOLDER` if absent.
- **W2 Link/Unlink commands + apply bodies:** `hierarchy/commands.rs` + crate-internal
  `EcsMaster::insert_one`/`remove_one` (extract from InsertCommand/RemoveCommand apply if not present).
- **W3 hook bodies + `has_parent`:** fill ChildOf on_insert/on_replace (link/unlink + guards), wire
  register_hooks. After W3: link/unlink/reparent consistent (no cascade yet).
- **W4 cascade despawn:** `Children::on_replace` (inline + wide paths) + CASCADE_SUPPRESS +
  `despawn_without_children` + `ClearChildrenCommand` + remove_children/clear_children. `despawn`
  becomes recursive.
- **W5 EntityCommands/Commands ergonomics + descent note:** add_child/add_children/set_parent/
  remove_parent/despawn_without_children + Commands::add_child; note (do NOT build) iter_descendants
  (descent = `Query<&Children>` + manual recursion for now).

## 9. Test plan outline
Link/unlink/reparent consistency (both directions); recursive despawn depth≥2 (all gone, no orphan
ChildOf/Children); despawn_without_children (children survive, ChildOf dangles — documented); self-ref
guard; dangling-parent guard; despawn-ordering #20106 (Children resolves to live children at cascade);
reparent atomicity (exactly one list post-drain); re-entrancy single-drain (3-level tree, one outermost
drain); clear_children/remove_children; **0%-gate bench (phase14a_hooks_gate flat — binding)**; Miri-TB
targets (ChildOf hooks enqueue path; Children::on_replace cascade BOTH paths = the F2 oracle; Link/Unlink
apply get_component_mut + structural insert_one/remove_one under the drain); property test (random
add/reparent/remove/despawn → invariant `c.ChildOf==p ⟺ p.Children∋c` for all live entities).

## Open questions for the critic
1. **`insert_one`/`remove_one` crate-internal primitive + inline-structural-op-mid-drain soundness.** §4
   does a first-child `Children` insert INLINE within `LinkChildCommand::apply` (under the held `&mut`,
   reusing InsertCommand machinery WITHOUT re-entering the deferred queue). Critic: confirm the
   inline structural op mid-drain is sound under the existing depth bracket, OR mandate routing the
   first-child `Children` insert through the deferred queue (one extra drain turn, simpler soundness).
   This is the single most likely place a forced `unsafe` / re-entrancy subtlety appears.
2. **`swap_remove` vs order-preserving** — confirm no CORE-phase child-order contract.
3. **`CASCADE_FANOUT_INLINE` cap (e.g. 32)** — stack-buffer size vs wide-path threshold; confirm/pick.

---

# Architect R2 Addendum — closes critic R2 (APPROVED once folded)

Direction unchanged; this is a delta. Net new `unsafe` for Phase 19 = **exactly one** (the M2
`MaybeUninit::assume_init` read) + one verbatim copy of an audited raw-deref pattern (C1 None-arm).

## C1 — primitive shape: Bundle-newtype for INSERT; `C`-keyed remove; NO `Children` remove
`Bundle` is sealed and all insert machinery is `B: Bundle`-bound, so a bare `Children` can't reuse it.
**Decision (option i):** private `#[derive(Bundle)] struct ChildrenBundle(Children);` +
`struct ChildOfBundle(ChildOf);` (1-field tuple Bundle derive is supported, `lib.rs:884-916`). INSERT
routes through the existing audited `merged_archetype_id::<ChildrenBundle>` + `migrate_entity_insert::
<ChildrenBundle>` verbatim → **zero new structural unsafe**; the "zero new unsafe" claim survives.
REMOVE is `C`-keyed (`RemoveCommand<ChildOf>` / `migrate_entity_remove<C>`) — no wrapper; and per W1 the
`Children`-remove path is deleted entirely (no `Children` remove authored).
- `migrate_entity_insert::<ChildrenBundle>` fires `on_add`+`on_insert` only (NOT `on_replace`,
  `migration_helpers.rs:446-473`); `Children` registers ONLY `on_replace` ⇒ inserting `Children` fires
  NO hook ⇒ **no spurious cascade** (Target-1(a) holds).
- `LinkChildCommand::apply(self, world)`: `if !world.has_entity(parent){return}`; `match
  world.get_component_mut::<Children>(parent) { Some(mut c)=>c.push(child), None=>{ /* first child */ let
  inland=world.entity_master.entities_inland[parent.id().0]; let src=unsafe{(*inland.archetype_ptr()).id()};
  let tgt=merged_archetype_id::<ChildrenBundle>(world,src); migrate_entity_insert::<ChildrenBundle>(world,
  parent,src,tgt,ChildrenBundle(Children::with_one(child))); } }`. The None-arm raw deref is a VERBATIM
  copy of the audited `insert_command.rs:74` F1 pattern (carries the same SAFETY; non-null+gen-matched by
  the preceding `has_entity`). Borrow note: `has_entity`→`get_component_mut`→`entities_inland[..]` are
  sequential exclusive borrows; the None arm holds nothing live across the migrate.
- `UnlinkChildCommand::apply`: `let Some(mut c)=world.get_component_mut::<Children>(parent) else {return};
  c.swap_remove_entity(child);` — NO remove-on-empty (W1).
- `ClearChildrenCommand::apply`: snapshot CURRENT `Children` to a stack buffer, drop the borrow, then per
  child route `RemoveCommand<ChildOf>(c)` (each fires `ChildOf::on_replace`→`UnlinkChild`; #17883-safe).
- Inline-mid-drain soundness (critic-proven) unchanged: the inline op IS `migrate_entity_insert` (the
  audited function); disjoint fields, stable archetype slab, depth≥1 no-drain — no new obligation.

## C2 — hand-written `component_id()` MUST trigger install (else hooks silently no-op)
`install_hooks::<C>` fires ONLY from inside `component_id()`. Mandate (W1) BOTH `ChildOf`/`Children`
hand-impl `Component` with the derive-equivalent body:
```rust
fn component_id() -> ComponentId {
    static ID: OnceLock<ComponentId> = OnceLock::new();
    *ID.get_or_init(|| {
        let raw = component_registry::register_new::<Self>();
        if Self::HAS_HOOKS { component_registry::install_hooks::<Self>(raw); }
        ComponentId(raw)
    })
}
const HAS_HOOKS: bool = true;
fn register_hooks(h: &mut ComponentHooks) { /* ChildOf: on_insert+on_replace; Children: on_replace */ }
```
**Install-probe test (§9):** `get_hooks(ChildOf::component_id().0)` → `Some`, `on_insert.is_some() &&
on_replace.is_some() && on_add.is_none() && on_remove.is_none()`; `get_hooks(Children::component_id().0)`
→ `Some`, `on_replace.is_some()` and the other three `is_none()`. (No existing test covers hand-impl
install; the negative asserts guard over-registration.)

## W1 — keep empty `Children` (option ii) — no archetype thrash
`UnlinkChildCommand::apply` does NOT remove `Children` when empty (R1 §4 remove-on-empty DELETED). An
ex-parent retains an empty `Children` (24 B, 0-cap Vec — no heap until next push). Rationale: a
child-count 0↔1↔0 oscillation under remove-on-empty would MIGRATE the parent archetype each transition
(~590 ns full byte-copy + EntityInland repoint + sibling unit_index perturbation) vs a pure in-place
`swap_remove_entity` (~90 ns class, zero structural op). 24 B retained is cheap; archetype-gated
iteration skips empty `Children` rows at zero cost. `Children` rustdoc documents the retained-empty.

## W2 — reparent atomicity, both paths (replaces §5(ii))
- **(a) Fresh `ChildOf`** (child had no parent): MIGRATING path fires `on_add`+`on_insert`, NO
  `on_replace` → `on_insert` enqueues `LinkChild(P,child)` (link-only, correct — no old parent).
- **(b) Overwrite `ChildOf`** (reparent A→B): `ChildOf` already in source ⇒ `apply_replace_in_place`
  fires `on_replace` (→`UnlinkChild(A)`) THEN `on_insert` (→`LinkChild(B)`); FIFO drain applies
  unlink-then-link, no user code interleaves → child ∈ B's list, ∉ A's. Ordering guaranteed ONLY because
  `ChildOf` is already in the source archetype. §9 reparent test covers BOTH.

## M1 — self-ref/dangling spurious `UnlinkChild` = apply-time no-op + non-alloc warn
Guard removes the rejected `ChildOf` → fires `on_replace` → enqueues `UnlinkChild(bad_parent,child)` for
a never-existent link → `apply` no-ops (`get_component_mut` None / `swap_remove_entity` false; no panic,
no mutation, no debug_assert). §9 asserts this. The guard's warn is `#[cold]` + `&'static str`,
**non-allocating** (no `format!`/`String`; id context only behind `#[cfg(debug_assertions)]`).

## M2 — `MaybeUninit` cascade buffer (no `Entity::PLACEHOLDER`)
`Entity::PLACEHOLDER` is NOT added. The cascade inline fast path uses
`let mut buf: [MaybeUninit<Entity>; CASCADE_FANOUT_INLINE] = [const { MaybeUninit::uninit() }; N];`,
`for i in 0..n { buf[i].write(children.as_slice()[i]); }`, drop the `&Children`, then read
`unsafe { buf[i].assume_init() }` (Entity is Copy) for `i in 0..n`. **This is the ONE new `unsafe`** —
SAFETY: `buf[..n]` initialized by the immediately-preceding write loop from a valid len-`n` slice; only
`[..n]` is read. The wide path re-derives per turn, no buffer, no unsafe.

## Open-question answers
1. C1 = option (i) (above). 2. `swap_remove`: NO sibling-order guarantee — `Children` rustdoc:
"Sibling order is unspecified and changes on removal; sort at the consumer if needed." 3. NO recursion
depth cap in CORE (queue-bounded, Bevy-parity); self-ref one-compare is the ONLY cycle protection; deep
`ChildOf` cycles are a documented footgun (DEFERRABLE: optional `#[cfg(debug_assertions)]` ancestor-walk
on set_parent — NOT Phase 19).

## Wave-breakdown delta (to §8)
- **W1:** define `ChildrenBundle`/`ChildOfBundle`; hand-`component_id()` uses the C2 body; DROP the
  `Entity::PLACEHOLDER` task (→ M2 MaybeUninit); add the C2 install-probe test.
- **W2:** REMOVE the `insert_one`/`remove_one` extraction (none authored). `LinkChildCommand::apply`
  calls `merged_archetype_id::<ChildrenBundle>`+`migrate_entity_insert::<ChildrenBundle>` directly;
  `UnlinkChildCommand::apply` = the W1 two-liner; `ClearChildrenCommand::apply` routes
  `RemoveCommand<ChildOf>` per child.
- **W3:** guard warn `#[cold]`+`&'static str` (M1).
- **W4:** cascade buffer = `MaybeUninit` (M2, the one `unsafe`); DELETE the R1 §4 remove-empty note;
  `Children` rustdoc gets no-order (Q2) + retained-empty (W1) + cycle-footgun (Q3) notes.
- **W5:** unchanged.
- **§9 additions:** C2 install-probe; reparent BOTH paths; self-ref/dangling spurious-`UnlinkChild` no-op.
- **§7 update:** zero new unsafe EXCEPT the one M2 `assume_init`; the C1 None-arm deref is an
  audited-pattern copy (insert_command.rs:74 F1 SAFETY); R1's insert_one/remove_one open-clause deleted.
