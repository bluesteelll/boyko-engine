> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Architecture: Phase 14a — Component Lifecycle Hooks

---

## §0 Readiness & framing

This phase adds `on_add` / `on_insert` / `on_replace` / `on_remove` / `on_despawn`
callbacks to component types. Unlike Phase 13 (`Local<T>`, zero hot-path risk),
this phase touches **six structural-op hot paths** and introduces a **reentrancy
soundness obligation** (a hook firing mid-`Command::apply` must not alias the
`&mut Archetype` / `&mut ComponentPool` the apply loop holds). The design is
therefore conservative: the entire mechanism is gated behind a single per-archetype
bit test that branch-predicts not-taken when no component in the archetype is
hooked, and structural changes from inside a hook are **forbidden in-place and
deferred** (Bevy + flecs's proven answer).

The load-bearing artifacts are (1) the `ArchetypeFlags` bit-test gate (§4, proves
0% regression), (2) the deferred-command reentrancy contract written against the
**actual** `command_queue.rs` apply loop (§5), and (3) the `DeferredEcsMaster`
hook-context view (§3). Every claim about boyko internals below was verified
against source at the file:line the research cited.

**Scope lock confirmed: 14a ships hooks only.** Full Observers (14b — entity-as-
observer machinery, `CachedObservers` reverse index, custom-event routing) are
deferred indefinitely per research §3. boyko's `EventDispatcher` is **not** an
observer backbone (research §6) and 14a does not couple to it.

---

## §1 Goal + scope

### Goal

Give component types innate, type-bound lifecycle callbacks fired synchronously at
the exact structural-op site, with **zero measurable cost when no hook is
registered**. Mirror Bevy's `ComponentHooks` semantics (the de-facto ECS standard)
adapted to boyko's type-erased `Command::apply` + per-system `CommandQueue`
architecture.

### Performance acceptance bar (the load-bearing gate)

| Bench | No-hook target | Rationale |
|---|---|---|
| `spawn_batch` warm path | **0% measurable regression** | Mirror Phase 10 "0% when unused". One `u16` load + `test`/`jz` per archetype boundary, not per row. |
| query iter (direct API) | **0% regression** | Hooks never touch the iteration path — only structural ops. |
| `Commands::spawn` single | **0% regression** | Bit test inside the cold dispatch fn; `if flags.is_empty()` short-circuits before any `Option` load. |
| insert / remove / despawn | **0% regression** | Same bit-test gate at each site. |

The bit-test gate must be proven by a criterion before any hook can fire (Wave 1
ships the gate + no-op dispatch and benches it; Waves 2-5 add the payload behind
the proven-free gate).

### In scope (14a deliverable)

1. `ArchetypeFlags` (`u16`) field on `Archetype` + one-time OR-compute at archetype
   construction.
2. `ComponentHooks` cold table — parallel `static HOOKS: [OnceLock<ComponentHooks>;
   MAX_COMPONENTS]` (keeps `ComponentLayout` at 56 B / one cache line).
3. `Component::HAS_HOOKS: bool` const + defaulted `register_hooks(&mut ComponentHooks)`.
4. `DeferredEcsMaster<'_>` hook-context view (mutate-not-restructure) + `HookContext`.
5. `#[cold] #[inline(never)]` `trigger_on_*` dispatch fns gated by
   `if archetype.flags.contains(...)`, wired into the 6 structural-op sites.
6. `#[derive(Component)]` `#[component(on_add = path, ...)]` attribute.
7. Runtime builder `EcsMaster::register_component_hooks::<C>()` with the
   "register before first use" constraint.
8. The 0%-regression bench gate.

### Explicitly out of scope

- **14b Observers** — entity-targeted observers, runtime add/remove of observers,
  `CachedObservers`, custom (non-lifecycle) events, the `On<E>`/`Trigger<E>` param.
- **Bundle-level hooks** — hooks are per-component; a bundle insert fires each
  component's hooks individually (Bevy parity).
- **`MaybeLocation` / `RelationshipHookMode`** — Bevy's `HookContext` carries a
  caller-location and a relationship mode; boyko has neither subsystem (research
  §1). `HookContext` is `{ entity, component_id }` only.
- **Parallel hook firing** — hooks fire ONLY in the single-threaded apply window
  (CQ7/APP4); never reachable from a parallel `&self` query path. This is a hard
  invariant, not a feature gap (§9 SAFETY-4).
- **Hook removal / re-registration after first use** — register-before-use only
  (§6, staleness rule).

---

## §2 Decisions

| ID | Decision | Choice | Justification | Rejected alternative |
|---|---|---|---|---|
| **Q1** | Hook context type | **(a) `DeferredEcsMaster<'_>`** — a `#[repr(transparent)]` newtype over `&mut EcsMaster` exposing component/resource mutation + a `deferred()` `Commands`-equivalent handle, statically withholding `create_entity` / `delete_entity` / `create_entity_at` / direct insert/remove. | Bevy-faithful and strictly more useful than Commands-only. The engineering cost is bounded: boyko already has every *read/mutate* method on `EcsMaster` (`get_component_mut`, `set_component_raw`, `insert_resource`, `remove_resource`) — the view is a thin forwarding wrapper that re-exposes the **safe subset** and adds one deferred-command handle. Withholding is achieved by the wrapper simply **not** forwarding the structural methods (there is no `Deref<Target = EcsMaster>` — that would leak everything). See §3.1 for the exact exposed/withheld split. | **(b) Commands-only** rejected: a hook frequently wants to read the dying value or mutate a sibling component (the canonical "on_remove: decrement a counter resource" / "on_add: initialize a derived field" patterns) — Commands-only forces those through a deferred round-trip, doubling latency and losing the synchronous-state guarantee. **(c) raw `&mut EcsMaster`** REJECTED — reentrancy UB (hook calling `create_entity` mid-apply aliases the live `&mut Archetype`). |
| **Q3** | Reentrancy / deferred dispatch | **Next-pass drain via the EXISTING `command_queue.rs` compaction block.** A hook enqueues structural commands into a **world-resident deferred queue** (`EcsMaster::deferred_hook_queue`, a new `CommandQueue` field reachable through `&mut EcsMaster`). The currently-draining `CommandQueue::apply` reads `stop_snapshot = bytes.len()` ONCE at entry; hook-enqueued commands land in a *different* queue and are applied by an explicit drain loop the scheduler/EcsMaster runs after the current queue's apply completes. | The originating per-system `Commands` queue is borrow-frozen on the stack frame **above** `Command::apply` — a hook cannot reach it. boyko has NO world-resident command queue today (verified: `ecs_master.rs` has no `CommandQueue` field). Adding one, reachable via `&mut EcsMaster`, is the minimal sound channel. **Next-pass** (not Bevy front-jump) because boyko's apply loop already snapshots `stop_snapshot` once and compacts post-loop pushes down for the *next* apply (lines 396, 489-509) — front-jump would require re-reading `bytes.len()` mid-loop, a behavior change to a Miri-clean panic-recovery path. See §5 for the full contract + soundness proof. | **Bevy front-jump (same pass)** rejected: would require modifying the `while local_cursor < stop_snapshot` bound to re-read `bytes.len()`, breaking the `CursorSync` RAII + `catch_unwind` survivor-range invariant that Phase 12.5/12.6 made Miri-clean. The next-pass drain reuses the proven machinery untouched. |
| **Q4** | `ArchetypeFlags` layout | **`flags: ArchetypeFlags(u16)` placed immediately after `signature`, before `arena`.** `u16` (5 hook bits now, 11 spare for 14b observer bits). | `offset_of!(Archetype, columns) == 0` MUST hold (asserted archetype.rs:170). Placing `flags` mid-struct (after `signature: ArchetypeSignature`, before `arena: *const Arena`) cannot disturb offset 0 — `columns: [Column; 512]` is the largest-align member and stays first under `#[repr(C)]` declaration order. `u16` is the smallest type giving 5 bits + observer headroom; it slots into existing padding between `signature` and the 8-byte-aligned `arena` pointer (zero size growth — the struct already pads to 8B alignment there). | `u32` (Bevy's choice) rejected — boyko needs only 5 + ~6 observer bits = 11, fits `u16`; `u32` wastes 2 B with no headroom benefit at boyko's bit count. `u8` rejected — 5 hook bits leaves only 3 for 14b, too tight. |
| **Q5** | Hook storage | **Parallel cold table `static HOOKS: [OnceLock<ComponentHooks>; MAX_COMPONENTS]`** mirroring `LAYOUTS`. | Inlining `ComponentHooks` (5 × `Option<HookFn>` = 40 B) into `ComponentLayout` pushes it from 56 B to 96 B — spills the one-cache-line guarantee that `component_registry.rs:90` documents and the hot `get_layout` read path relies on. The parallel table keeps `ComponentLayout` at 56 B; `HOOKS[id]` is touched ONLY during archetype construction (cold) to compute flags, never on the hot read path. | Inline-in-`ComponentLayout` rejected — breaks the 56 B / 64 B cache-line invariant for a table read on every memcpy. |
| **Q7** | `on_replace` on in-place insert | **Fire `on_replace` THEN `on_insert`** in `apply_replace_in_place` (Bevy parity). | `apply_replace_in_place` (insert_command.rs:164-168) does `drop_at(row)` → `write_at(row, bytes)` per component. Bevy semantics: replacing an existing component value fires `on_replace` (old value about to be overwritten) before the drop, then `on_insert` after the write (new value present). This is the only semantically correct ordering — a hook tracking "value changed" needs both edges. Diverging would surprise every user who knows Bevy. `on_add` does NOT fire (the component was already present — add means *newly* present). | Only-`on_insert` rejected — loses the `on_replace` edge that mirrors Bevy and that `on_remove`-symmetric bookkeeping needs. |
| **Q8** | Panic policy | **Caught-and-aborts-the-command, propagated by the existing `catch_unwind`.** A hook panic unwinds exactly like a `Command::apply` panic: `consume_and_drop_glue` already advanced the cursor past the command (W3'), the `CursorSync` guard syncs the cursor on unwind, the outer `catch_unwind` captures survivors into `panic_recovery`, and `resume_unwind` propagates. The hook's structural side-effects were deferred (Q3) and never executed, so no partial structural state leaks. | Aligns with boyko's existing Command-panic semantics (Phase 12.5/12.6) AND the panic-in-Drop policy (component.rs:11-28): a hook is user code reached during apply, so it inherits the apply window's panic-recovery contract exactly. A hook that panics is a logic bug; the frame's command queue recovers its survivors and the panic surfaces to the application boundary. NO new `catch_unwind` is added (that would cost ~20-30 ns/op and pollute the hot path). | **Abort process** rejected — too severe for recoverable user-code panic; boyko already recovers Command panics. **Catch-and-log-continue** rejected — silently swallowing a hook panic hides logic bugs and diverges from the Command-panic contract (which propagates). |
| **REG** | Registration API | **Both: derive attribute (primary) + runtime builder (secondary).** Derive `#[component(on_add = path, ...)]` generates `const HAS_HOOKS = true` + a `register_hooks` impl; the macro-generated `component_id()` installs hooks into `HOOKS[id]` on first call. Runtime `EcsMaster::register_component_hooks::<C>() -> ComponentHooksBuilder<'_>` for non-derive registration, gated by the register-before-use rule. | Derive is the ergonomic default and matches `#[derive(Component)]`'s existing `OnceLock`-backed `component_id()` codegen. The runtime builder covers foreign types / dynamic registration. The register-before-use rule (§6) makes `ArchetypeFlags` correct: hooks must be installed before the component first appears in any archetype, else the archetype's flags are stale and the hook is silently skipped (research §7 staleness hazard). | Derive-only rejected — no path for foreign component types. Runtime-only rejected — loses the zero-boilerplate derive ergonomics. |

---

## §3 Data structures

### §3.1 `DeferredEcsMaster<'w>` — the hook-context view (Q1)

```rust
/// Restricted view of `EcsMaster` handed to lifecycle hooks.
///
/// Exposes component/resource MUTATION and a deferred-command handle, but
/// statically WITHHOLDS every structural-change method (no `create_entity`,
/// `delete_entity`, `create_entity_at`, no direct insert/remove). A hook
/// firing mid-`Command::apply` therefore cannot alias the `&mut Archetype` /
/// `&mut ComponentPool` the apply loop is concurrently writing (§9 SAFETY-1).
///
/// # Why a wrapper, not `Deref`
///
/// There is intentionally NO `Deref<Target = EcsMaster>` — that would leak the
/// full (structural) API. The view forwards ONLY the safe subset listed below.
#[repr(transparent)]
pub struct DeferredEcsMaster<'w> {
    // Raw NonNull, NOT &mut, to keep the apply loop's *mut Archetype
    // reborrows from being invalidated under Tree Borrows (the view is
    // minted from the same &mut EcsMaster the apply loop holds; see §9
    // SAFETY-1 for the non-overlap argument).
    world: NonNull<EcsMaster>,
    _marker: PhantomData<&'w mut EcsMaster>,
}

impl<'w> DeferredEcsMaster<'w> {
    // ── EXPOSED (sound during apply) ──────────────────────────────────────
    /// Read a component of a (possibly different) entity.
    pub fn get_component<T: Component>(&self, e: Entity) -> Option<&T>;
    /// Mutate a component. SAFE during apply ONLY for components NOT in the
    /// row the apply loop is currently writing (§9 SAFETY-1 documents the
    /// non-aliasing obligation; in practice hooks mutate siblings/resources).
    pub fn get_component_mut<T: Component>(&mut self, e: Entity) -> Option<&mut T>;
    /// Read/insert/remove a resource — resources live OUTSIDE archetype
    /// storage, so this never aliases the apply loop's component writes.
    pub fn resource<R: Resource>(&self) -> Option<&R>;
    pub fn resource_mut<R: Resource>(&mut self) -> Option<&mut R>;
    /// Enqueue a structural command into the world-resident deferred queue
    /// (Q3). Applied AFTER the current queue's apply completes — never inline.
    pub fn commands(&mut self) -> DeferredCommands<'_>;

    // ── WITHHELD (NOT forwarded — compile error if a hook tries) ──────────
    //   create_entity, create_entity_at, delete_entity, spawn_*, the direct
    //   insert/remove paths. A hook that needs these uses `commands()`.
}
```

**Exposed/withheld split (exact):**

| Method | Exposed? | Reason |
|---|---|---|
| `get_component<T>` / `get_component_mut<T>` | ✅ | Component mutation is sound for non-apply-row components; the canonical hook use. |
| `resource<R>` / `resource_mut<R>` | ✅ | Resources are outside archetype storage — never alias apply writes. |
| `commands()` → `DeferredCommands` | ✅ | The deferred channel for structural change (Q3). |
| `current_tick` (read) | ✅ | Hooks may need the tick for change-detection-aware logic. |
| `create_entity` / `create_entity_at` | ❌ | Structural — would alias the apply loop's `&mut Archetype`. |
| `delete_entity` | ❌ | Structural — would `swap_remove` under the apply loop. |
| `spawn_one` / `spawn_two` / `spawn_batch` | ❌ | Structural. |
| `register_component_hooks` | ❌ | Registration during apply violates register-before-use (§6). |

### §3.2 `ComponentHooks` + `HookFn` (Q5)

```rust
/// Type-erased lifecycle-hook function pointer.
///
/// Mirrors the `DropFn = unsafe fn(*mut u8)` precedent (component_registry.rs:67)
/// — a plain fn pointer, zero-alloc, monomorphized at registration. `unsafe`
/// because the dispatch site guarantees the apply-window-only + non-aliasing
/// invariants the body relies on (§9).
///
/// # Safety (caller — always a `trigger_on_*` dispatch fn)
/// - Invoked ONLY inside the single-threaded `CommandQueue::apply` window
///   (CQ7/APP4) — never from a parallel `&self` path.
/// - `DeferredEcsMaster` is minted from the SAME `&mut EcsMaster` the apply
///   loop holds; the hook must not mutate the component row the apply loop is
///   concurrently writing (§9 SAFETY-1).
/// - `ctx.entity` is live; `ctx.component_id` names a component the entity has.
pub type HookFn = unsafe fn(DeferredEcsMaster<'_>, HookContext);

/// Per-`ComponentId` lifecycle hooks. Stored in the parallel cold `HOOKS`
/// table (Q5), NOT inline in `ComponentLayout` (keeps the latter at 56 B).
///
/// `None` slots are zero-cost: "is this kind hooked?" == `Option::is_some()`,
/// the same pattern `ComponentLayout::drop_fn` uses. All-`None` is the default
/// for any component without a `#[component(...)]` attribute / runtime builder.
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct ComponentHooks {
    pub on_add:     Option<HookFn>,  // 8 B (niche-optimized)
    pub on_insert:  Option<HookFn>,  // 8 B
    pub on_replace: Option<HookFn>,  // 8 B
    pub on_remove:  Option<HookFn>,  // 8 B
    pub on_despawn: Option<HookFn>,  // 8 B
}
// 40 B total. Lives in HOOKS[id] (cold); never on the hot read path.

/// Context passed to every hook. Bevy's `MaybeLocation` / `RelationshipHookMode`
/// are omitted — boyko has neither subsystem (research §1).
#[derive(Clone, Copy)]
#[repr(C)]
pub struct HookContext {
    pub entity: Entity,            // 8 B — the entity the op targets
    pub component_id: ComponentId, // 8 B — which component triggered the hook
}
// 16 B.
```

### §3.3 `ArchetypeFlags` (Q4)

```rust
/// Per-archetype "which hook kinds does ANY component in this archetype
/// declare?" bitset. OR-computed once at archetype construction; read as a
/// single `u16` load + `test`/`jz` on every structural-op dispatch (the
/// no-hook branch predicts not-taken).
///
/// Mirrors Bevy's `ArchetypeFlags: u32` (research §1) at boyko's bit count.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
#[repr(transparent)]
pub struct ArchetypeFlags(u16);

impl ArchetypeFlags {
    pub const ON_ADD_HOOK:     u16 = 1 << 0;
    pub const ON_INSERT_HOOK:  u16 = 1 << 1;
    pub const ON_REPLACE_HOOK: u16 = 1 << 2;
    pub const ON_REMOVE_HOOK:  u16 = 1 << 3;
    pub const ON_DESPAWN_HOOK: u16 = 1 << 4;
    // bits 5..16 reserved for 14b observer flags.

    #[inline] pub const fn empty() -> Self { Self(0) }
    #[inline] pub const fn contains(self, bit: u16) -> bool { self.0 & bit != 0 }
    #[inline] pub fn insert(&mut self, bit: u16) { self.0 |= bit; }
    #[inline] pub const fn is_empty(self) -> bool { self.0 == 0 }
}
```

**`Archetype` field placement (Q4):**

```rust
#[repr(C)]
pub struct Archetype {
    pub(crate) columns: [Column; MAX_COMPONENTS], // offset 0 — UNCHANGED, asserted
    pub(crate) id: ArchetypeId,
    pub(crate) component_pools: ComponentPoolBundle,
    pub(crate) current_index: usize,
    pub(crate) signature: ArchetypeSignature,
    pub(crate) flags: ArchetypeFlags,             // NEW — 2 B in existing padding
    pub(crate) arena: *const Arena,                // 8B-aligned; flags fits before it
    pub(crate) component_ids: Vec<ComponentId>,
    pub(crate) entity_ids: Vec<EntityId>,
}
const _: () = assert!(std::mem::offset_of!(Archetype, columns) == 0); // STILL HOLDS
```

### §3.4 `EcsMaster` deferred-hook queue field (Q3)

```rust
// NEW field on EcsMaster — the world-resident channel a hook's
// DeferredCommands enqueues into. Reachable via &mut EcsMaster (which the
// apply loop holds), unlike the borrow-frozen per-system Commands queue.
//
// Lazy: Vec::new() inside CommandQueue is allocation-free until first push
// (command_queue.rs:87). A world with no hooks that enqueue commands pays
// zero allocation. Drop-order: declared AFTER arena (commands may hold
// arena-derived bytes), mirroring query_state_cache's C5 placement.
pub(crate) deferred_hook_queue: CommandQueue,
```

---

## §4 Dispatch design (the `#[cold]` gated trigger fns)

### §4.1 Trigger function shape

Each of the five hook kinds gets a `#[cold] #[inline(never)]` trigger fn. The
**cheap flag check is the caller's guard**, mirroring Bevy's `bundle.rs` pattern
(research §1: "the cheap flag check is *inside*"). The trigger fn itself is cold
because it is only entered when a hook actually exists.

```rust
// In a new module: crates/boyko_ecs/src/ecs/core/component/hooks/dispatch.rs

/// Fire `on_add` for `component_id` on `entity`. Cold: only called when the
/// archetype's ON_ADD_HOOK bit is set (the caller's gate).
#[cold]
#[inline(never)]
pub(crate) fn trigger_on_add(
    world: NonNull<EcsMaster>,
    component_id: ComponentId,
    entity: Entity,
) {
    // HOOKS[id] read is cold (archetype already proved SOME component is
    // hooked; this confirms it is THIS component). One Acquire load + branch.
    if let Some(hooks) = component_registry::get_hooks(component_id)
        && let Some(f) = hooks.on_add
    {
        // SAFETY (§9 SAFETY-1, -4): minted from the &mut EcsMaster the apply
        //   loop holds; apply-window-only; the hook withholds structural ops
        //   via DeferredEcsMaster. ctx.entity is live (just pushed/migrated).
        let view = unsafe { DeferredEcsMaster::from_world(world) };
        let ctx = HookContext { entity, component_id };
        // SAFETY (HookFn contract): see DeferredEcsMaster + apply-window invariant.
        unsafe { f(view, ctx); }
    }
}
// trigger_on_insert / trigger_on_replace / trigger_on_remove / trigger_on_despawn
// are byte-identical modulo the `hooks.on_X` field selected.
```

### §4.2 The gate at each call site (the 0%-regression mechanism)

The hot-path cost in the no-hook case is, at each structural-op site:

```rust
// archetype.flags is a u16 already in cache (the archetype was just touched
// for the row write). This is ONE load + ONE test/jz, predicted not-taken.
if archetype.flags.contains(ArchetypeFlags::ON_ADD_HOOK) {
    trigger_on_add(world_ptr, component_id, entity); // cold, never inlined
}
```

When `flags.is_empty()` (no component in the archetype declares any hook —
overwhelmingly the common case), the whole hook block compiles to a few
`test`/`jz` instructions that the branch predictor learns immediately. No
`HOOKS` table load, no `Option` deref, no function call. This is the Phase 10
"0% when unused" mechanism applied to structural ops.

### §4.3 The six structural-op sites (research §5 table)

| Lifecycle | Site (file:line verified) | Hooks fired | When |
|---|---|---|---|
| spawn (deferred) | `SpawnAtCommand::apply` (spawn_at_command.rs:240-247) | `on_add` + `on_insert` for ALL components | AFTER the write loop + `register_entity_with_ptr` |
| spawn (direct) | `EcsMaster::create_entity` (ecs_master.rs:599) / `create_entity_at` (697) | `on_add` + `on_insert` for ALL | AFTER `register_entity_with_ptr` |
| insert (in-place) | `InsertCommand::apply_replace_in_place` (insert_command.rs:164-168) | `on_replace` (pre-`drop_at`) + `on_insert` (post-`write_at`) | per component |
| insert (migration) | `migrate_entity_insert` (migration_helpers.rs:401-404) | `on_add` (newly-added only) + `on_insert` (all bundle comps) | AFTER `EntityInland` repointed to target |
| remove | `migrate_entity_remove` (migration_helpers.rs:495-506) | `on_replace` + `on_remove` for C | BEFORE `drop_at(source_row)` (line 505) |
| despawn | `delete_entity` (ecs_master.rs:905-906) | `on_replace` + `on_remove` for ALL | BEFORE `archetype.remove_entity` (line 906) |

### §4.4 Spawn site code sketch (the add+insert bookend)

```rust
// In SpawnAtCommand::apply, AFTER Step 7 (register_entity_with_ptr, line 247):
//
// archetype is the &mut Archetype the apply loop wrote into; we already have
// its flags in cache. world_ptr is NonNull::from(&mut *world).
//
// Ordering invariant (§9 SAFETY-2): ALL on_add fire, THEN ALL on_insert
// (Bevy: add-before-insert across the whole bundle, not interleaved).
if !archetype.flags.is_empty() {
    let world_ptr = NonNull::from(&mut *world);
    let entity = self.entity; // captured before the bundle was consumed
    if archetype.flags.contains(ArchetypeFlags::ON_ADD_HOOK) {
        for &cid in archetype.component_ids() {
            trigger_on_add(world_ptr, cid, entity);
        }
    }
    if archetype.flags.contains(ArchetypeFlags::ON_INSERT_HOOK) {
        for &cid in archetype.component_ids() {
            trigger_on_insert(world_ptr, cid, entity);
        }
    }
    // After the hooks return, drain any structural commands they enqueued
    // into world.deferred_hook_queue (Q3 — next-pass via the caller; see §5).
}
```

Note: `archetype` must be re-borrowed as needed; the hooks receive `world_ptr`
(not `&mut archetype`), so the `&mut Archetype` reborrow is dropped before any
hook runs — eliminating the alias between the hook's `DeferredEcsMaster` and the
apply loop's archetype borrow (§9 SAFETY-1).

### §4.5 Despawn site code sketch (the replace+remove pre-drop bookend)

```rust
// In EcsMaster::delete_entity, BEFORE archetype.remove_entity (line 906).
// The hooks fire PRE-DROP so they can still read the dying component values
// (Bevy fires on_replace/on_remove before the bytes are dropped — research §5).
//
// `archetype` here is `&mut *inland.archetype_ptr()` (line 905). We mint
// world_ptr and DROP the &mut archetype borrow before firing hooks.
let flags = archetype.flags;          // copy out (u16) before dropping borrow
let comp_ids = if !flags.is_empty() { // cheap: only clone when hooked
    Some(archetype.component_ids().to_vec()) // cold path only
} else { None };
// ... &mut archetype borrow ends here (NLL) ...

if let Some(comp_ids) = comp_ids {
    let world_ptr = NonNull::from(&mut *self);
    if flags.contains(ArchetypeFlags::ON_REPLACE_HOOK) {
        for &cid in &comp_ids { trigger_on_replace(world_ptr, cid, entity); }
    }
    if flags.contains(ArchetypeFlags::ON_REMOVE_HOOK) {
        for &cid in &comp_ids { trigger_on_remove(world_ptr, cid, entity); }
    }
}

// NOW re-resolve the archetype and proceed with the existing removal:
let archetype: &mut Archetype = unsafe { &mut *inland.archetype_ptr() };
let outcome = archetype.remove_entity(removed_unit_index); // line 906 — drops bytes
// ... existing RemoveOutcome handling ...
```

The `component_ids().to_vec()` allocation is on the **cold** path (only when
`flags` is non-empty) — it never touches the no-hook hot path. (A
stack-`SmallVec`-style fixed buffer is a follow-up micro-opt; correctness first.)

### §4.6 ArchetypeFlags compute site (Q4)

`create_by_ids` (archetype.rs:226-229) and `register_component_inplace`
(archetype.rs:281-284) already iterate `component_ids`. Extend the loop to OR each
component's hook flags:

```rust
// In create_by_ids, inside/after the existing `for &comp_id in component_ids` loop:
let mut flags = ArchetypeFlags::empty();
for &comp_id in component_ids {
    archetype.component_pools.add_pool(arena, comp_id);
    archetype.refresh_column(comp_id);
    flags.insert_from_hooks(comp_id); // reads HOOKS[comp_id] once (cold, construction)
}
archetype.flags = flags;

// insert_from_hooks: a free fn / ArchetypeFlags method:
//   if let Some(h) = get_hooks(cid) {
//       if h.on_add.is_some()     { self.insert(ON_ADD_HOOK); }
//       ... etc for all 5 ...
//   }
```

`register_component_inplace` (used by the bundle-cache slab construction path) must
do the same OR for the single added component. **Staleness rule (§6):** because
hooks are read here at construction, a hook registered AFTER an archetype exists is
NOT reflected — register-before-use is mandatory.

---

## §5 Reentrancy model (Q3) — the soundness crux

### §5.1 The hazard

A hook firing mid-`SpawnAtCommand::apply` that calls `world.create_entity(...)`
would:
1. Reborrow `&mut Archetype` while the apply loop holds a live `*mut Archetype`
   reborrow → aliasing UB.
2. Potentially `Vec::resize` `entities_inland` / `sparse_to_active` while the apply
   loop holds raw pointers into them → use-after-realloc UB.

This is exactly the UB Bevy's `DeferredWorld` and flecs's defer counter prevent.

### §5.2 The contract

1. **Hooks cannot perform structural changes inline.** `DeferredEcsMaster`
   (§3.1) withholds every structural method. A hook that wants to spawn/insert/
   remove/despawn calls `ctx.commands()` → `DeferredCommands`, which `push`es a
   `Command` into `world.deferred_hook_queue` (§3.4).

2. **Hook-enqueued commands drain NEXT pass, not inline.** The
   `deferred_hook_queue` is a *separate* `CommandQueue` from the per-system queue
   currently in `apply`. After each structural-op's hook block returns, the
   driver (the site itself, or the scheduler's apply window — §5.4) calls
   `world.drain_deferred_hook_queue()`, which runs
   `deferred_hook_queue.apply(world)`. Because that is a *fresh* `apply` call on a
   *different* queue, its `stop_snapshot` is read fresh and the existing machinery
   handles it correctly.

3. **Re-entrant enqueue is bounded by the existing compaction block.** If a
   command applied from `deferred_hook_queue` itself fires a hook that enqueues
   MORE commands, those land past that drain's `stop_snapshot` and are compacted
   down for the *next* drain iteration (command_queue.rs:489-509 — the proven
   Q-A1.1 case-4 fix). The driver loops `while !deferred_hook_queue.is_empty()`
   until quiescent (bounded by the user's hook graph being acyclic; a cyclic
   hook-spawns-hooked-entity graph would loop — documented as a user-error, same
   class as infinite recursion).

### §5.3 Soundness against the real apply loop

Verified against `command_queue.rs:341-510`:

- The apply loop reads `stop_snapshot = self.bytes.as_ref().len()` ONCE (line
  348) and loops `while local_cursor < stop_snapshot` (line 396). **Commands a
  hook enqueues into `deferred_hook_queue` do not touch `self.bytes` at all** —
  different queue, different allocation. So the current walk's bound is
  untouched; no mid-loop `bytes.len()` re-read is needed (we explicitly do NOT
  do Bevy front-jump).
- The `CursorSync` RAII guard (lines 291-319, 391-394) syncs `local_cursor` into
  the persistent cursor on normal exit AND unwind. A hook panic unwinds through
  `consume_and_drop_glue` → the guard fires → survivors captured. **Unchanged** —
  the hook is invoked from inside `cmd.apply`, which is inside the
  `catch_unwind`-wrapped walk (line 240-242). A hook panic is indistinguishable
  from a command-body panic to the recovery machinery (Q8).
- The post-loop compaction block (lines 489-509) handles the per-system queue's
  own command-during-apply pushes (if a system's command body itself enqueues —
  orthogonal to hooks). Hooks never push into the per-system queue (it is
  borrow-frozen above `apply`), so this block's behavior is unchanged by Phase
  14a.

### §5.4 Where the drain is driven

Two drive points, both under the same `&mut EcsMaster`:

- **Direct API** (`EcsMaster::create_entity` / `delete_entity`): the method drains
  `deferred_hook_queue` before returning (the caller holds `&mut self`, no borrow
  conflict).
- **Command apply** (`SpawnAtCommand` / `InsertCommand` / `RemoveCommand` /
  `DespawnCommand`): each command's `apply` body drains `deferred_hook_queue` at
  its end, before returning to the per-system queue's apply loop. This keeps the
  hook-enqueued commands applied within the same logical flush, after the
  triggering command, in queue order.

**SAFETY**: the drain calls `deferred_hook_queue.apply(world)` which needs `&mut
EcsMaster`. Inside a `Command::apply` we have `world: &mut EcsMaster`. We mint a
raw-twin drain (mirroring `CommandQueue::raw`) to avoid double-`&mut` on the queue
field — `deferred_hook_queue` is a field of `world`, so `world.deferred_hook_queue.apply(world)`
would alias. Resolution: a `EcsMaster::drain_deferred_hook_queue(&mut self)` that
`mem::take`s the queue's bytes into a local, applies the local against `self`, then
restores capacity (the same `mem::take` + reborrow pattern `CommandQueue::Drop`
uses at lines 711-712). Detailed in §7 Step 5.

---

## §6 Registration (REG) + the staleness rule

### §6.1 Derive attribute (primary)

```rust
#[derive(Component)]
#[component(on_add = my_on_add, on_remove = my_on_remove)]
struct Health(u32);

// where:
unsafe fn my_on_add(world: DeferredEcsMaster<'_>, ctx: HookContext) { /* ... */ }
```

The macro (boyko_macros/src/lib.rs:31-84) is extended to:
1. Parse the optional `#[component(...)]` attribute via `parse_nested_meta` (the
   `#[event]` macro already uses this idiom, lib.rs:288).
2. Emit `const HAS_HOOKS: bool = true;` (else the trait default `false`).
3. Emit a `register_hooks(&mut ComponentHooks)` impl assigning the parsed paths.
4. **Wire install into `component_id()`**: the existing `OnceLock`-backed
   `component_id()` (lib.rs:55-61) is extended so the `get_or_init` closure, after
   `register_new::<Self>()`, calls `component_registry::install_hooks::<Self>(id)`
   which writes `HOOKS[id]` from `Self::register_hooks`. This guarantees hooks are
   installed atomically with ID assignment, before the component can appear in any
   archetype (the archetype-construction flag compute reads `HOOKS[id]`).

### §6.2 `Component` trait widening (backward-compatible)

```rust
pub trait Component: 'static + Sized {
    fn component_id() -> ComponentId;

    /// Compile-time elision flag. `false` by default — components without a
    /// `#[component(...)]` attribute pay zero. Enables `if const { C::HAS_HOOKS }`
    /// in monomorphic typed paths (research §7 secondary layer).
    const HAS_HOOKS: bool = false;

    /// Install this component's hooks into the supplied table. Defaulted empty;
    /// derive / runtime builder override it. Called once at `component_id()`
    /// init (§6.1).
    #[inline]
    fn register_hooks(_hooks: &mut ComponentHooks) {}

    // ... existing defaulted helpers unchanged ...
}
```

Both additions are backward-compatible widenings (a const with a default + a
defaulted method) — every existing `#[derive(Component)]` and hand-impl keeps
compiling with `HAS_HOOKS = false` and an empty `register_hooks`.

### §6.3 Runtime builder (secondary)

```rust
impl EcsMaster {
    /// Register hooks for `C` at runtime. MUST be called before `C` first
    /// appears in any archetype (§6.4 staleness rule) — debug-asserted.
    pub fn register_component_hooks<C: Component>(&mut self) -> ComponentHooksBuilder {
        // Forces C::component_id() (mints id + installs derive hooks if any),
        // then returns a builder writing into HOOKS[id]. Panics in debug if
        // any live archetype already contains C (stale-flag guard).
    }
}

// Usage:
world.register_component_hooks::<Health>()
     .on_add(my_on_add)
     .on_remove(my_on_remove);
```

### §6.4 The staleness rule (research §7 hazard)

**Rule: hooks for a component MUST be registered before that component first
appears in any archetype.** `ArchetypeFlags` is computed once at archetype
construction by reading `HOOKS[id]` (§4.6). A hook installed *after* an archetype
containing the component already exists would leave that archetype's flag bit
unset → the hook is **silently skipped** for entities in that archetype.

Mitigations:
- `register_component_hooks::<C>()` debug-asserts no live archetype contains `C`
  (`ArchetypeMaster` scan, cold). In release it is a no-op (the user owns the
  contract, matching boyko's `register_layout` collision-detection philosophy).
- The derive path is staleness-immune by construction: hooks install at
  `component_id()` first-call, which always precedes the first archetype
  containing the component (archetype construction calls `component_id()` to get
  the ID).
- Documented prominently in the public `register_component_hooks` rustdoc and the
  mdBook hooks page.

---

## §7 Wave / step plan

Each wave compiles independently and passes `cargo test --all-targets`. Waves 1
proves the 0%-regression gate before any hook payload exists.

### Wave 1 — `ArchetypeFlags` + compute + no-op dispatch scaffolding (proves 0%)
1. **Step 1** — `ArchetypeFlags(u16)` type (new file
   `core/component/hooks/archetype_flags.rs`) + the 5 bit consts + `contains` /
   `insert` / `is_empty`. Unit tests for bit ops.
2. **Step 2** — add `flags: ArchetypeFlags` to `Archetype` (§3.3); re-assert
   `offset_of!(columns) == 0`; initialize `flags = ArchetypeFlags::empty()` in
   `Archetype::new` / `create_by_ids` / the slab-construction path. (Compute is
   no-op until Wave 2 lands `HOOKS`.)
3. **Step 3** — wire the **no-op** gate at all 6 sites: `if
   !archetype.flags.is_empty() { /* empty block */ }`. Since `flags` is always
   empty until Wave 2, this is the pure-overhead measurement.
4. **Step 4 (bench gate)** — run the spawn_batch / query iter / insert / despawn
   benches. **MUST show 0% regression** vs the pre-Wave-1 baseline. This proves
   the gate is free before any payload is added. (If not 0%, the design is wrong —
   stop and revisit.)

### Wave 2 — `ComponentHooks` table + `HAS_HOOKS` + install
5. **Step 5** — `ComponentHooks` + `HookContext` + `HookFn` (§3.2) in
   `core/component/hooks/mod.rs`. `static HOOKS: [OnceLock<ComponentHooks>;
   MAX_COMPONENTS]` + `get_hooks(id)` / `install_hooks::<C>(id)` in
   `component_registry.rs` (mirroring `LAYOUTS` / `register_new`). Re-assert
   `ComponentLayout` is still 56 B.
6. **Step 6** — `Component::HAS_HOOKS` const + defaulted `register_hooks` (§6.2).
7. **Step 7** — `ArchetypeFlags::insert_from_hooks(cid)` + wire it into the
   `create_by_ids` + `register_component_inplace` compute loops (§4.6). Now flags
   reflect registered hooks. Unit test: archetype with a hooked component has the
   right bit; archetype without has empty flags.

### Wave 3 — `DeferredEcsMaster` view + deferred queue (Q1 + Q3 plumbing)
8. **Step 8** — `EcsMaster::deferred_hook_queue: CommandQueue` field (§3.4) +
   `drain_deferred_hook_queue(&mut self)` (the `mem::take` + raw-twin apply
   pattern, §5.4). Drop-order placement after `arena`.
9. **Step 9** — `DeferredEcsMaster<'w>` (§3.1) + `DeferredCommands<'_>` (wraps a
   `&mut CommandQueue` pointing at `deferred_hook_queue`, exposing `spawn` /
   `entity(e).insert/remove/despawn` that `push` into it). Unit test: a command
   enqueued via `DeferredCommands` lands in `deferred_hook_queue` and applies on
   drain.

### Wave 4 — trigger fns + wire dispatch into the 6 sites
10. **Step 10** — the five `#[cold] #[inline(never)]` `trigger_on_*` fns (§4.1) in
    `core/component/hooks/dispatch.rs`.
11. **Step 11** — replace the Wave-1 no-op gate blocks with real `trigger_on_*`
    calls at the spawn sites (`SpawnAtCommand::apply`, `create_entity`,
    `create_entity_at`, `migrate_entity_insert`) — add+insert, with the drain
    call after (§4.4, §5.4).
12. **Step 12** — wire the insert-replace site (`apply_replace_in_place`):
    `on_replace` pre-`drop_at`, `on_insert` post-`write_at`, per component (Q7).
13. **Step 13** — wire the remove + despawn sites (`migrate_entity_remove`
    pre-line-505; `delete_entity` pre-line-906) — replace+remove PRE-DROP (§4.5).
    Drain after.

### Wave 5 — derive macro attribute + runtime builder
14. **Step 14** — `#[component(on_add = ..., ...)]` parsing + `HAS_HOOKS` +
    `register_hooks` codegen + install-into-`component_id()` wiring (§6.1) in
    `boyko_macros/src/lib.rs`. trybuild tests for malformed attributes.
15. **Step 15** — `EcsMaster::register_component_hooks::<C>()` +
    `ComponentHooksBuilder` + the staleness debug-assert (§6.3, §6.4).

### Wave 6 — tests + the 0%-regression re-gate
16. **Step 16** — the full test suite (§8).
17. **Step 17 (final bench gate)** — re-run the 0%-regression benches WITH the full
    mechanism present but NO hooks registered. MUST still be 0% (the gate is the
    same; the payload is dormant). Then a *separate* bench measures the cold-path
    cost of an archetype WITH hooks (informational, not a gate).

---

## §8 Test plan (architecture-level)

### Per-kind firing (unit + integration)
- **spawn fires add+insert**: spawn a hooked entity → `on_add` then `on_insert`
  observed once each (a hook increments a thread-local / resource counter).
- **despawn fires remove for all**: despawn → `on_replace` + `on_remove` fired for
  every component, BEFORE drop (the hook reads the dying value successfully — a
  hook that reads `ctx`'s component via `get_component` sees the still-live bytes).
- **insert-replace fires on_replace+on_insert** (not on_add): in-place insert over
  an existing component → `on_replace` then `on_insert`, `on_add` NOT fired (Q7).
- **insert-migration fires add+insert**: insert that changes archetype → `on_add`
  for newly-added components, `on_insert` for all bundle components.
- **remove fires on_remove pre-drop**: a hook that reads the dying value via
  `DeferredEcsMaster::get_component` succeeds (proves PRE-drop ordering, §4.5).

### Ordering
- **add-before-insert across a bundle**: a 3-component bundle fires all three
  `on_add` before any `on_insert` (§4.4 — not interleaved per-component).

### Deferred reentrancy (Q3 — the soundness test)
- **hook spawns → deferred, applied next**: an `on_add` hook that calls
  `ctx.commands().spawn(bundle)` → the spawn is enqueued, NOT executed inline; the
  new entity exists AFTER `drain_deferred_hook_queue`. No UB (verified by the test
  passing under Miri, below).
- **hook despawns the triggering entity**: an `on_insert` that despawns its own
  entity → deferred; the entity is gone after drain; no double-free.
- **re-entrant hook chain quiesces**: hook A spawns a hooked entity whose `on_add`
  hook B enqueues nothing → drain loop terminates (bounded).

### 0%-regression bench (THE load-bearing gate)
- `spawn_batch_10k`, `query_iter_10k`, `commands_spawn_single`,
  `despawn_10k` — each compared with-mechanism-no-hooks vs pre-Phase-14a baseline.
  Acceptance: within criterion noise (±5%), mirroring Phase 10.

### Miri
- `cargo +nightly miri test` on: the dispatch path (a hook that mutates a sibling
  component), the deferred-command path (hook enqueues → drain), and the
  pre-drop remove path (hook reads dying value). Single-threaded (multi-thread
  Miri deferred per the project's Phase 9.1 precedent — hooks fire only in the
  single-threaded apply window anyway, §9 SAFETY-4).

### Multi-hook + no-hook
- **multi-hook component**: a component with all 5 hooks → each fires at its site.
- **no-hook component**: a component with no attribute → `HAS_HOOKS == false`,
  archetype flags empty, no trigger fn entered (assert via a hook-side counter
  that stays 0).
- **mixed archetype**: archetype with one hooked + one non-hooked component → only
  the hooked component's hooks fire.

---

## §9 SAFETY invariants

- **SAFETY-1 (reentrancy / aliasing — paramount)**: a hook's `DeferredEcsMaster`
  is minted from the same `&mut EcsMaster` the apply loop holds. Soundness rests
  on: (a) `DeferredEcsMaster` withholds ALL structural methods (§3.1), so a hook
  cannot reborrow `&mut Archetype` or resize `entities_inland`; (b) the triggering
  site drops its `&mut Archetype` reborrow BEFORE firing hooks (§4.4, §4.5 — hooks
  receive `world_ptr`, not `&mut archetype`); (c) a hook mutating a component must
  not target the exact row the apply loop is mid-write on — in practice hooks
  mutate siblings/resources/the dying value (read-only), and the
  `get_component_mut` path resolves a fresh pointer each call (no cached aliasing
  pointer survives). Structural change is deferred (Q3).
- **SAFETY-2 (ordering — add before insert; replace+remove pre-drop)**: spawn
  fires all `on_add` then all `on_insert`; remove/despawn fire `on_replace` +
  `on_remove` BEFORE `drop_at` / `remove_entity` so the hook reads live bytes.
  Enforced by the call-site ordering (§4.4, §4.5).
- **SAFETY-3 (`ArchetypeFlags` correctness)**: `flags` is the exact OR of every
  contained component's hook presence, computed at construction from `HOOKS[id]`.
  Correctness requires register-before-use (§6.4) — a stale flag silently skips a
  hook (a missed callback, NOT memory unsafety; the worst case is a logic bug, not
  UB).
- **SAFETY-4 (apply-window-only firing)**: hooks fire ONLY inside the
  single-threaded `CommandQueue::apply` window (CQ7/APP4) or the single-threaded
  direct `create_entity`/`delete_entity` path — both under exclusive `&mut
  EcsMaster`. They are NEVER reachable from a parallel `&self` query path (Phase 9
  workers never call structural ops). This is why `HookFn` taking `&mut`-derived
  world access is sound: there is no concurrent reader.
- **SAFETY-5 (deferred-queue drain non-aliasing)**: `drain_deferred_hook_queue`
  `mem::take`s the queue bytes into a local before applying against `&mut self`,
  so `world.deferred_hook_queue` is not aliased by the `apply(world)` call (mirrors
  `CommandQueue::Drop` lines 711-712).
- **SAFETY-6 (panic during apply)**: a hook panic unwinds through the existing
  `consume_and_drop_glue` W3' cursor-advance + `CursorSync` guard + outer
  `catch_unwind`; survivors recover, the panic propagates (Q8). No new
  `catch_unwind` is introduced.

Every `unsafe` block in the implementation carries a `// SAFETY:` comment citing
the relevant invariant above.

---

## §10 Risk register

| Risk (research §7 + design) | Severity | Resolution |
|---|---|---|
| **Reentrancy UB** — hook does structural change inline | Critical | `DeferredEcsMaster` withholds structural methods; all structural change deferred to `deferred_hook_queue`, drained next-pass (Q3, §5). The (c) raw-`&mut EcsMaster` signature is the trap — rejected. |
| **Drop-order** — hook reads already-dropped bytes | High | Fire `on_replace`/`on_remove` PRE-`drop_at`/`remove_entity` (§4.5). Tested (the dying-value-read test, §8). |
| **`ArchetypeFlags` staleness** — hook registered post-archetype silently skipped | Medium | Register-before-use rule (§6.4); derive path is staleness-immune; runtime builder debug-asserts no live archetype contains C. A miss is a logic bug, not UB. |
| **0%-regression claim fails** — gate is not actually free | High | Wave 1 ships the gate + no-op dispatch and benches it BEFORE any payload (Step 4). If not 0%, the design is rejected and revisited before proceeding. The gate is one `u16` load + `test`/`jz`, identical to Phase 10's proven-free `if const` pattern. |
| **`DeferredEcsMaster` soundness** — the view leaks an aliasing path | High | No `Deref`; only the safe subset forwarded (§3.1). The `get_component_mut` non-aliasing obligation documented (SAFETY-1c) + Miri-tested. |
| **Re-entrant hook chain non-termination** — hook spawns hooked entity ad infinitum | Low | Documented as user-error (same class as infinite recursion); the drain loop is bounded by an acyclic hook graph. Not a soundness issue (each iteration is a fresh, sound apply). |
| **`Added<T>` overlap** (DOTS lesson) — on_add duplicates change-detection | Low (scope) | Documented: some `on_add` needs are already served by Phase 10 `Added<T>` at zero structural-op cost. The hooks page recommends `Added<T>` for query-side reactions and hooks only for synchronous structural-op-time side effects. |
| **Macro attribute parse errors** | Low | trybuild tests for malformed `#[component(...)]` (unknown key, missing path). |

---

## §11 Integration

### Affected modules
- `core/component/component.rs` — `HAS_HOOKS` const + `register_hooks` (widening).
- `core/component/component_registry.rs` — `HOOKS` table + `get_hooks` /
  `install_hooks`.
- `core/component/hooks/` (NEW) — `archetype_flags.rs`, `mod.rs` (`ComponentHooks`,
  `HookContext`, `HookFn`), `dispatch.rs` (`trigger_on_*`),
  `deferred_master.rs` (`DeferredEcsMaster`, `DeferredCommands`).
- `core/archetype/archetype.rs` — `flags` field + compute in `create_by_ids` /
  `register_component_inplace`.
- `core/ecs_master/ecs_master.rs` — `deferred_hook_queue` field,
  `drain_deferred_hook_queue`, `register_component_hooks`, hook dispatch at
  `create_entity` / `create_entity_at` / `delete_entity`.
- `core/commands/{spawn_at,insert,remove,despawn}_command.rs` +
  `migration_helpers.rs` — hook dispatch + drain at each apply site.
- `boyko_macros/src/lib.rs` — `#[component(...)]` attribute parsing + codegen.

### No changes to
- `ComponentPool` / `Chunk` / `Arena` (hooks operate above storage).
- The parallel scheduler (hooks fire only in the apply window — SAFETY-4).
- The query iteration path (hooks never touch it — 0%-regression guarantee).
- `EventDispatcher` (decoupled per research §6).

### Compatibility with `Arena` / `ComponentPool` / `UnitId`
- Verified: hooks read/mutate via the existing `get_component_raw_mut` path
  (resolves a fresh pointer per call — no cached aliasing). The deferred queue
  stores `Command` bytes (Send + 'static) exactly like the per-system queue.
  `Archetype` stays `Send + Sync` (the `flags: u16` is trivially so).

---

## §12 Open questions

None blocking. Two deliberately-deferred follow-ups, documented as such:
- **Cold-path `to_vec()` in the despawn site (§4.5)** — replaced by a stack
  fixed-buffer in a follow-up micro-opt; correctness-first here, and it is cold
  (only when `flags` non-empty).
- **`if const { C::HAS_HOOKS }` in typed monomorphic paths** — the secondary
  compile-time layer (research §7) helps only the direct typed `spawn_one`/
  `spawn_two` paths; most ops go through type-erased apply where the runtime
  `ArchetypeFlags` is the load-bearing gate. Wired opportunistically where a
  monomorphic path exists; not a gate.

---

## §13 Readiness statement

**Ready for the architecture-critic.**

The three areas where I most want the critic's scrutiny:

1. **Q1 `DeferredEcsMaster` soundness (§3.1 + SAFETY-1)** — the view is minted from
   the same `&mut EcsMaster` the apply loop holds. I argue soundness rests on (a)
   withholding structural methods, (b) dropping the `&mut Archetype` reborrow
   before firing hooks, and (c) the `get_component_mut` non-aliasing obligation. The
   critic should stress whether (c) is a *static* guarantee or a *documented
   obligation* the user can violate — and whether `get_component_mut` returning a
   `&mut T` into archetype storage that the just-completed apply wrote is provably
   non-overlapping with any live pointer.

2. **Q3 reentrancy contract against the real apply loop (§5)** — I chose
   **next-pass drain via a separate world-resident queue** over Bevy front-jump,
   specifically to avoid modifying the Miri-clean `while local_cursor <
   stop_snapshot` + `CursorSync` + `catch_unwind` machinery. The critic should
   verify that `drain_deferred_hook_queue`'s `mem::take` + raw-twin-apply pattern
   is sound when invoked from *inside* a `Command::apply` (i.e. nested apply on a
   different queue under the same `&mut EcsMaster`), and that the re-entrant
   enqueue → compaction path (§5.2 #3) actually terminates and never double-applies.

3. **The 0%-regression claim (§1 bar + Wave 1 Step 4)** — I assert the `u16` flag
   load + `test`/`jz` at six sites is free in the no-hook case, proven by benching
   the no-op gate in Wave 1 before any payload exists. The critic should challenge
   whether the despawn-site `flags`-copy + conditional `to_vec()` (§4.5) truly
   compiles away in the empty-flags case, and whether adding the `flags` field
   perturbs `Archetype`'s layout/cache behavior on the spawn hot path (it should
   not — it lands in existing padding, but the critic should demand the layout
   assertion + a size tripwire).
