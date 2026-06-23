# Architecture: Generic Relations API (`Relationship` / `RelationshipTarget`)

Status: PLAN (pre-implementation). Author: architect. Branch: `ecs`.
Supersedes the "Relations design PARKED" note from the Option/AnyOf campaign.

## Critic round 1 — resolutions (changelog)

Architecture-critic returned CHANGES REQUESTED on round 1; the core was CONFIRMED sound (Option-A
refactor; BUG-P19-TB-1 cascade generalizes correctly because `DeferredEcsMaster` exposes no
`&mut`-into-storage; B5 cascade-site correct; 0%-gate + no-fragmentation preserved). All findings
ACCEPTED. Resolutions applied below:

- **C1 (phantom trait bound)** — Dropped the `Component<Mutability = Mutable>` bound from BOTH
  `Relationship` and `RelationshipTarget`. boyko's `Component` has NO `Mutability` associated type and
  no `Mutable` marker (confirmed against source). The `*_risky` mutator naming fence is boyko's
  existing substitute for Bevy's mutability-typestate, so the bound was vacuous. NO
  `Component::Mutability` kernel change is in scope. Traits are now
  `pub trait Relationship: Component` / `pub trait RelationshipTarget: Component + Default`.
- **C2 (entity-remap metadata)** — (a) The derive computes `LAYOUT_FINGERPRINT` GENERICALLY from the
  actual relationship struct (repr tag from the struct's own `#[repr(...)]`, `offset_of!(Self, field)`,
  `size_of::<Self>()`, `size_of::<Entity>()`), NOT by copying `ChildOf`'s hard-coded transparent /
  offset-0 value. (b) Added clone-remap + serialize-remap TRIPWIRES for the DERIVED `Likes` to the
  R2 matrix (the in-crate `ChildOf` hand-mirror does not exercise the derive's auto-emit path).
- **B3 (non-existent-target guard)** — The generic guard uses the view's actual liveness method.
  `DeferredEcsMaster` exposes `has_parent(Entity) -> bool` (already a generic liveness check
  delegating to `EcsMaster::has_entity`); it is RENAMED to `is_alive(Entity) -> bool` for generic
  use (pure rename, same body), with a `#[doc(hidden)]` `has_parent` retained for the Phase-19
  hand-mirror call site. The pseudocode's `view.has_entity(...)` is corrected to `view.is_alive(...)`.
- **W1 + O3 (defer new re-entrant surfaces to v1.1)** — v1 ships with `RETAIN_EMPTY = true` MANDATORY
  for every relationship target. `RETAIN_EMPTY = false` (remove-on-empty) AND the 1:1 `Entity`-collection
  + eviction path are DEFERRED to v1.1. Rationale: both are NEW re-entrant edges (remove-on-empty fires
  the target's `on_replace`; 1:1 eviction fires `try_remove`); shipping them in v1 would double the
  Miri-TB audit surface. v1 introduces EXACTLY ONE new re-entrant surface — the generalized cascade —
  already covered by the full Phase-19 Miri-TB suite.
- **W2 (cascade soundness = STRUCTURAL)** — Reframed cascade soundness as a structural argument, not a
  documented contract: (1) a hook's only world handle is `DeferredEcsMaster`, which exposes NO
  `&mut`-into-storage (`get_component` → `&T` only); (2) the `*_risky` mutators require `&mut Self`,
  obtainable only inside a `Command::apply` under `&mut EcsMaster`, NEVER inside a hook. Added a
  compile-fail test (R5) asserting a hook body cannot obtain `&mut`-into a `RelationshipTarget`
  collection.
- **W3 (target install-probe)** — Added R2 install-probe `likedby_registers_only_on_replace` asserting
  the `RelationshipTarget` derive set EXACTLY `on_replace` (B7 spurious-first-cascade guard).
- **W4 (cycle guard RELEASE-active for LINKED_DESPAWN)** — The cascade depth-bound guard is now
  RELEASE-active for `LINKED_DESPAWN` relations (cycles are normal data for a generic API). A depth
  counter (reusing the `MAX_PROPAGATION_DEPTH` pattern from `trigger_walk`) terminates the cascade with
  a `#[cold]` warn past the bound — one increment per cascade level on the already-cold despawn path,
  ZERO cost on the 0%-gate and for non-`LINKED_DESPAWN` relations. Added R6 (cyclic-LINKED_DESPAWN
  terminates).

## Goal

Lift the bespoke Phase-19 `ChildOf` / `Children` machinery into a **generic, monomorphized
trait pair** (`Relationship` / `RelationshipTarget`) plus a `RelationshipSourceCollection`
container abstraction and two `#[derive(...)]` macros, so that ANY user struct can declare a
one-to-many (or 1:1) bidirectional relation maintained by the existing component-hook substrate
— with **zero new runtime cost** and **`ChildOf`/`Children` becoming one specialization of the
generic code** (D4 = Option A, refactor; justified below).

Performance contract (all must hold post-refactor, measured against the Phase-19 baseline):
- **0%-when-unused**: a program that mints no relationship component ids leaves the cold `HOOKS`
  slots unset → the per-archetype `ArchetypeFlags` gate (one `u16` load + predict-not-taken `jz`)
  raises no relationship bit → hot iteration is asm-identical. No relationship code on any path
  that does not touch a relationship component.
- **No new archetype fragmentation**: relationships fragment by *type* exactly as `ChildOf` /
  `Children` do today; no per-value fragmentation, no extra component on non-participants. The
  retain-empty rule (avoid `0↔1↔0` migration thrash, ~590 ns byte-copy vs ~90 ns `swap_remove`)
  is preserved as a per-target **policy const**, not hard-coded.
- **Relate is O(1)**: insert pushes the source into the target collection (no dedup scan) — the
  source-of-truth asymmetry from Phase-19 is preserved.
- **Cascade soundness (BUG-P19-TB-1) is preserved structurally**: relationship-maintenance hooks
  ONLY enqueue into `deferred_hook_queue`; they never mutate storage inline. The
  `apply_via_raw_twin` disjoint-allocation walk (the fix) remains the sole drain path. No hook in
  this design re-opens the Tree-Borrows hazard.
- **Monomorphization is direct**: every generic hook body resolves at compile time to a concrete
  `unsafe fn(DeferredEcsMaster, HookContext)` — a bare fn pointer in the `HOOKS` table. No
  `dyn`, no vtable, no `TypeId` dispatch on the hot path. Generic `<R: Relationship>::on_insert`
  instantiates one fn per relationship type, exactly as a hand-written hook.

Principle 0 compliance: the reverse index lives in a normal `ComponentPool` column (the target
component), not a side `HashMap`. The container is a per-row owned `Vec<Entity>` today, behind a
trait that permits a future dense/arena backing without touching the hooks.

## Context and constraints

Subsystems touched: `boyko_ecs` component/hooks, `hierarchy/` (refactor target),
`component/observers/traversal.rs` (generic bubbling), `component_registry` (no new install kind
needed — see §"Integration"), `boyko_macros` (two new derives). No changes to the scheduler,
query, archetype storage, or `EntityMaster`.

Invariants that MUST be preserved (verified against source):
- The `HookFn = unsafe fn(DeferredEcsMaster<'_>, HookContext)` shape and the 5-slot
  `ComponentHooks { on_add, on_insert, on_replace, on_remove, on_despawn }` are unchanged.
- The C2 tripwire: a `component_id()` (hand- or derive-written) that sets `HAS_HOOKS = true`
  MUST call `install_hooks::<Self>(raw)`, else the cold slot stays unset and hooks silently
  never fire. The macro already emits this; our generic path rides it.
- `ChildOf` carries entity-remap clone + serialize metadata (`CloneViaFn` + `child_of_map_entities`
  + `SerializeViaFn` + `child_of_load_map_entities` + `LAYOUT_FINGERPRINT`). `Children` is
  `Cloneability::Ignore` (rebuilt via `LinkChildCommand` on deep-clone). Any generalization MUST
  preserve these or deep-clone / load-remap silently breaks (the existing tripwire tests guard it).
- The bidirectional global invariant `c.ChildOf == p ⟺ p.Children ∋ c` (the proptest oracle).
- The cascade fires from `delete_entity_core` via `Children::on_replace` reading the CURRENT
  collection (NOT Bevy's `on_despawn` site — boyko's kernel funnels despawn cleanup through the
  per-component pre-remove `on_replace`). This is a structural fact of our kernel; the generic
  design adapts to it (see D4 mapping table).

Target metrics: zero added cycles/cache-misses/allocations on the no-relationship path
(asm-identical to today); a new `Likes`/`LikedBy` relation must pass the same proptest invariant
and the same Miri-TB cascade gate as `ChildOf`/`Children`.

---

## Key decisions

### Decision 1: The trait pair — `Relationship` + `RelationshipTarget`

**What**: Two `Component`-bound traits. `Relationship` is the writable source-of-truth foreign
key (holds one `Entity`); `RelationshipTarget` is the derived reverse index (holds a
`Collection` of source entities). The bidirectional sync is done entirely by generic
component-hook bodies keyed on the trait — no bespoke per-relation code.

```rust
/// The source-of-truth side: a foreign key on the source entity pointing at one target.
/// Implemented (via derive) on the component the user writes (e.g. `ChildOf`).
pub trait Relationship: Component + Sized {
    /// The reverse-index component on the target entity.
    type Target: RelationshipTarget<Source = Self>;

    /// Read the target `Entity` out of `self`. Monomorphized to a single field load.
    fn get(&self) -> Entity;

    /// Construct from a target `Entity` (other fields via `Default`). Used by ergonomics.
    fn from(target: Entity) -> Self;

    /// Generic LINK hook: on insert, push `source` into the target's collection.
    /// Default body = the generic algorithm (§Algorithms). The derive wires this into
    /// `hooks.on_insert`. NOT overridable by the user (the relation owns the slot).
    unsafe fn on_insert(view: DeferredEcsMaster<'_>, ctx: HookContext);

    /// Generic UNLINK hook: on replace (overwrite / remove / pre-despawn), remove `source`
    /// from the OLD target's collection. The derive wires this into `hooks.on_replace`.
    unsafe fn on_replace(view: DeferredEcsMaster<'_>, ctx: HookContext);
}

/// The derived reverse index. User code must NEVER write it directly; the `*_risky`
/// methods are the only mutators and exist solely for the hooks/ergonomics.
pub trait RelationshipTarget: Component + Default + Sized {
    /// The source-of-truth component.
    type Source: Relationship<Target = Self>;
    /// The container of source entities.
    type Collection: RelationshipSourceCollection;

    /// `true` → despawning the target recursively despawns all sources (Bevy `linked_spawn`).
    /// `ChildOf`/`Children` set this `true`.
    const LINKED_DESPAWN: bool;

    /// `true` → keep an emptied collection (do NOT remove the target component) to dodge
    /// `0↔1↔0` archetype-migration thrash. `Children` sets this `true` (the Phase-19 perf rule).
    /// **v1: MUST be `true` for every target.** `RETAIN_EMPTY = false` (remove-on-empty, the Bevy
    /// default) is a NEW re-entrant edge (it fires the target's own `on_replace` on emptying) and is
    /// DEFERRED to v1.1 — see W1/O3. The const stays in the trait so v1.1 lifts the restriction
    /// without an API break.
    const RETAIN_EMPTY: bool;

    fn collection(&self) -> &Self::Collection;
    /// `_risky`: bypasses the source-of-truth invariant — hooks/ergonomics ONLY.
    fn collection_mut_risky(&mut self) -> &mut Self::Collection;
    fn from_collection_risky(c: Self::Collection) -> Self;

    /// Generic CASCADE hook: drives source cleanup / recursive despawn. The derive wires this
    /// into `hooks.on_replace` (matching boyko's pre-remove despawn site, NOT `on_despawn`).
    unsafe fn on_replace(view: DeferredEcsMaster<'_>, ctx: HookContext);

    #[inline] fn with_capacity(cap: usize) -> Self {
        Self::from_collection_risky(<Self::Collection>::with_capacity(cap))
    }
    #[inline] fn len(&self) -> usize { self.collection().len() }
    #[inline] fn is_empty(&self) -> bool { self.collection().is_empty() }
}
```

**Why**: This is the Bevy-0.16 proven shape (the named precedent), but bound to boyko's `HookFn`
shape. (No `Mutability = Mutable` bound: boyko's `Component` has no `Mutability` associated type —
the `*_risky` naming fence is boyko's substitute for Bevy's mutability typestate, so the bound is
vacuous; C1.) The source-of-truth
asymmetry gives O(1) relate (push, no dedup scan). Monomorphization: `<R>::on_insert` /
`<R>::on_replace` / `<T>::on_replace` each instantiate one concrete fn per relation type — a bare
fn pointer in `HOOKS`, identical to today's hand-written `child_of_on_insert`. Zero dispatch
overhead, no `dyn`.

**Alternatives rejected**:
- *flecs-style first-class `(Relation, Target)` entity-id pairs with value fragmenting*: would
  rebuild archetype storage to fragment by relationship value — a kernel rewrite, archetype
  explosion under many distinct targets, violates "no new fragmentation". Rejected.
- *Single `Relation` trait carrying both directions*: collapses the source-of-truth asymmetry,
  forces a dedup scan or ambiguity about which side is writable. Rejected.

**Trade-off**: The `*_risky` naming is the only fence preventing user code from corrupting the
reverse index (Rust visibility can't express "private mutator, public reader" across the trait).
Same fence Bevy uses; acceptable.

### Decision 2: `RelationshipSourceCollection` — the container abstraction

**What**: A trait abstracting the reverse-index container, so the cardinality (1:many vs 1:1) and
backing (Vec today, dense later) are a *type* choice, not new traits.

```rust
pub trait RelationshipSourceCollection {
    type Iter<'a>: Iterator<Item = Entity> where Self: 'a;

    fn with_capacity(cap: usize) -> Self;
    fn reserve(&mut self, additional: usize);
    /// Push `e`. Returns `true` iff newly added (set-backed collections dedup; Vec always `true`).
    fn add(&mut self, e: Entity) -> bool;
    /// Remove `e`. Returns `true` iff it was present.
    fn remove(&mut self, e: Entity) -> bool;
    fn iter(&self) -> Self::Iter<'_>;
    fn len(&self) -> usize;
    fn clear(&mut self);
    /// 1:1 only: `Some(prev)` iff a prior source must be evicted before `add` (the `Entity`
    /// impl returns the held entity; `Vec` returns `None`). Drives 1:1 re-target eviction.
    #[inline] fn source_to_evict_before_add(&self) -> Option<Entity> { None }
    #[inline] fn is_empty(&self) -> bool { self.len() == 0 }
}
```

Provided impls (v1):
- `Vec<Entity>` — the one-to-many default (what `Children` uses). No dedup; `add` always `true`.

Deferred to v1.1 (W1/O3):
- `Entity` — the one-to-one collection (capacity 1). `add` evicts via `source_to_evict_before_add`
  returning the held entity; the generic `on_insert` then `try_remove`s the prior source's
  `Relationship`. This is a NEW re-entrant edge (eviction fires `try_remove`); deferring it keeps
  v1's Miri-TB audit surface to EXACTLY ONE new re-entrant edge (the generalized cascade). The
  `source_to_evict_before_add` hook on the collection trait stays RESERVED (default `None`) so 1:1
  is a clean v1.1 add with no trait change.

**Why**: Cardinality-via-collection-type (the cleanest Bevy extensibility point) keeps the trait
pair fixed while admitting 1:1 now and dense/M:N later. `Vec<Entity>` is a per-row owned container
in a normal `ComponentPool` column — Principle-0 compliant (no side store). A dense/arena backing
is a future `impl` that does not touch any hook body.

**Alternatives rejected**: `SmallVec` inline-storage for the common small-fanout case — explicitly
NOT chosen in Phase-19 (the `Children` Vec header is 24 B, zero-cap empty = no heap; SmallVec
inline would bloat every parent row and complicate the `repr(transparent)` clone/serialize
fingerprint). Defer until a fanout-distribution measurement justifies it. **Open question O1.**

**Trade-off**: `Vec` does not dedup — double-relating the same pair adds the source twice. A
set-backed collection is the opt-in fix; for `ChildOf` this matches today's behavior (the proptest
generator never double-adds, and `add_child` is the only path).

### Decision 3: D4 = Option A (refactor `ChildOf`/`Children` onto the generic machinery)

**Decision: Option A. Refactor.** See the dedicated D4 section below for the full justification and
the Phase-19-behavior → generic-policy mapping table.

### Decision 4: Two derive macros, riding the existing hook-codegen path

**What**: `#[derive(Relationship)]` and `#[derive(RelationshipTarget)]`, each a thin layer that
(a) emits the trait `impl`, and (b) reuses the *existing* `ComponentHookPaths::codegen` machinery
(`boyko_macros/lib.rs:426`) to wire the generic hook bodies into `register_hooks`. The component
itself is still declared via `#[derive(Component)]`; the relationship derive only adds the trait
impl and the hook wiring. (Spec in §"Derive-macro spec".)

**Why**: The macro crate *already* turns attribute-supplied hook paths into a `register_hooks`
body + `const HAS_HOOKS = true` (`ComponentHookPaths::codegen`), and already emits the full
`const HAS_X + register_X + gated install_X::<Self>(raw)` triple (the `#[require]` plumbing). The
relationship derive composes these proven patterns — it does NOT invent a new install kind. The
generic hook is `<Self as Relationship>::on_insert`, a fully-resolved fn path the existing codegen
assigns into `hooks.on_insert`.

**Alternatives rejected**: a standalone `#[derive(Relationship)]` decoupled from `Component` —
Bevy explicitly warns this lets a type be a `Relationship` without `Component`, silently not
installing hooks. We keep the derive an *additive* layer over `#[derive(Component)]` and document
the pairing (and a compile-fail test guards a `Relationship` derive whose hooks would collide with
a user `#[component(on_insert=...)]`).

**Trade-off**: The user writes two derives (`#[derive(Component, Relationship)]`). Acceptable —
it's explicit and mirrors the existing `#[derive(Component)]` + attribute ergonomics.

### Decision 5: Generic event bubbling — reuse the existing `Traversal` trait

**What**: The generic `Traversal` trait already exists (`observers/traversal.rs:19`) and is NOT
hardcoded to `ChildOf` — `trigger_walk` calls `E::Traversal::next` per hop (`ecs_master.rs:3275`).
Add ONE blanket-style bridge so any `Relationship` bubbles for free:

```rust
/// Bubble toward the target of any single-target relationship `R`.
pub struct Toward<R: Relationship>(PhantomData<R>);
impl<R: Relationship> Traversal for Toward<R> {
    #[inline]
    fn next(view: &DeferredEcsMaster<'_>, current: Entity) -> Option<Entity> {
        view.get_component::<R>(current).map(|r| r.get())
    }
}
```

`ChildOfTraversal` becomes a type alias `= Toward<ChildOf>` (or is kept and reimplemented via
`get`). A custom trigger bubbles along a non-`ChildOf` relation by setting
`type Traversal = Toward<MyRel>`.

**Why**: The bubbling machinery is already generic; we only add the relationship→traversal bridge.
No change to `trigger_walk`, the propagate TLS, or `MAX_PROPAGATION_DEPTH`. The custom-trigger-
along-a-non-ChildOf-relation requirement is satisfied by this one type. (Verified: `Trigger`'s
`type Traversal: Traversal` associated type already drives the hop.)

**Trade-off**: None beyond the existing per-hop `get_component` cost (one column lookup), already
paid by `ChildOfTraversal` today.

---

## D4 DECISION: Option A (refactor) — justification + Phase-19 → generic-policy mapping

**Decision: Option A — refactor `ChildOf`/`Children` to BE the generic machinery.** `ChildOf` becomes
`#[derive(Component, Relationship)] #[relationship(target = Children)]`; `Children` becomes
`#[derive(Component, RelationshipTarget)] #[relationship_target(source = ChildOf, linked_despawn, retain_empty)]`.
The hand-written `child_of_on_insert` / `child_of_on_replace` / `children_on_replace` bodies are
**deleted** and replaced by the generic `<R>::on_insert` / `<R>::on_replace` / `<T>::on_replace`
bodies they become specializations of.

**Why A over C (conform-only)**:
1. **Principle 0 + DRY-soundness**: Option C (a second copy of the link/unlink/cascade logic living
   alongside the bespoke `ChildOf` code, merely *conforming* to the traits) is the exact "parallel
   data system glued on the side" anti-pattern the project root-caused as the SP4 race. Two copies
   of the cascade = two places the BUG-P19-TB-1 disjoint-allocation discipline can drift. One
   generic body, exercised by both `ChildOf` and the new `Likes` relation, means the Miri-TB
   cascade gate covers ALL relationships at once. This is the project's stated lesson (one generic
   kernel feature, used uniformly).
2. **Bevy proves subsumption is total**: in Bevy 0.16 `ChildOf`/`Children` are *pure derives* with
   zero bespoke relationship logic. The generic bodies are strictly a generalization of boyko's
   hand-written ones (the research confirms a body-by-body correspondence). There is no `ChildOf`
   behavior the generic machinery cannot express once `RETAIN_EMPTY` and the cascade-site choice
   are policy.
3. **Zero perf cost**: monomorphizing `<ChildOf as Relationship>::on_insert` produces the same
   machine code as today's `child_of_on_insert` (same reads, same guards, same deferred command).
   The asm-identical hot path and the 0%-gate are unaffected — the install hub, `HAS_HOOKS`, and
   the `ArchetypeFlags` bits are unchanged.

**Why NOT C**: C keeps the bespoke bodies forever, makes the traits a documentation veneer, and
doubles the soundness-audit surface for every future cascade change. The only thing C buys is "no
churn in `hierarchy/`" — but the churn is mechanical (delete the bodies, add two derives, keep the
clone/serialize metadata) and is fully covered by the Phase-19 regression suite as the gate.

**Risk + mitigation**: the refactor must preserve `ChildOf`'s clone/serialize entity-remap metadata
(`CloneViaFn` + `child_of_map_entities` + `SerializeViaFn` + `child_of_load_map_entities` +
`LAYOUT_FINGERPRINT`). The generic `#[derive(Relationship)]` **auto-emits the entity-remap clone +
serialize metadata for the single `Entity` field** (a relationship's foreign key is *by definition*
an `Entity` that must be remapped on clone/load) — this is mandatory, not optional (Bevy folds it
in for the same reason). `Children` keeps `Cloneability::Ignore`. The existing tripwire tests
(`clone_install_*`, `hooks_install_*`) plus the full Phase-19 suite are the gate.

### Phase-19 behavior → generic policy mapping

| # | Phase-19 behavior (source) | Generic policy in the new design | Notes |
|---|---|---|---|
| B1 | `ChildOf` single-target foreign key, `#[repr(transparent)] Entity` | `Relationship`, `Collection = Vec<Entity>` on `Children` (one-to-many) | `get()` = field load; `repr` unchanged |
| B2 | `Children(Vec<Entity>)` derived reverse index, user-read-only | `RelationshipTarget`, `Collection = Vec<Entity>`; mutators are `*_risky` | privacy fence preserved |
| B3 | `child_of_on_insert`: self-ref guard, dangling-parent guard, else `LinkChildCommand` | `<R>::on_insert` generic body: self-ref guard (`target==source`→remove R), dangling guard (`!view.is_alive(target)`→remove R, via the renamed liveness method — B3), else enqueue link command | byte-for-byte same guards + same deferred command |
| B4 | `child_of_on_replace`: copy old parent, enqueue `UnlinkChildCommand` | `<R>::on_replace` generic body: read old target, enqueue unlink | same |
| B5 | `children_on_replace`: cascade (inline ≤32 / wide path), reads CURRENT collection, `cascade_suppressed()` early-out | `<T>::on_replace` generic body, parameterized by `LINKED_DESPAWN`; same inline/wide split via `CASCADE_FANOUT_INLINE` | cascade hung on `on_replace` (boyko site), NOT `on_despawn` |
| B6 | Reparent A→B = `on_replace(A)` then `on_insert(B)`, FIFO drain → atomic move | unchanged — same hook firing order, same two deferred commands | proptest oracle covers it |
| B7 | First-child insert fires NO spurious cascade (`Children` registers only `on_replace`, not `on_add`/`on_insert`) | `RelationshipTarget` derive wires ONLY `on_replace`; never `on_add`/`on_insert` | preserved by the derive's slot selection |
| B8 | Keep-empty: emptied `Children` retained (avoid `0↔1↔0` thrash) | `const RETAIN_EMPTY: bool = true` on `Children`; generic unlink/cascade honor it | the perf rule becomes an explicit policy const |
| B9 | Recursive despawn = default; opt-out `despawn_without_children` + `CascadeSuppressGuard` | `LINKED_DESPAWN = true`; suppress guard + `cascade_suppressed()` early-out unchanged (kernel-level, not per-relation) | the suppress TLS stays in `commands.rs`, relation-agnostic |
| B10 | `ChildOf` clone = `CloneViaFn` + `child_of_map_entities` remap | generic `#[derive(Relationship)]` auto-emits `CloneViaFn` + entity-remap for the foreign-key field | mandatory; tripwire-tested |
| B11 | `ChildOf` serialize = `SerializeViaFn` + `WireBridge` + `child_of_load_map_entities` + `LAYOUT_FINGERPRINT` | generic derive auto-emits `SerializeViaFn` + load-remap + fingerprint for the foreign-key field | mandatory; tripwire-tested |
| B12 | `Children` clone = `Cloneability::Ignore` (rebuilt via Link commands) | `RelationshipTarget` derive emits `Cloneability::Ignore` | reverse index never byte-copied |
| B13 | Self-ref guard = single compare; deep cycles NOT detected | preserved; **add a RELEASE-active depth-bound guard for `LINKED_DESPAWN` relations** (W4; see EC7): terminates with a `#[cold]` warn past `MAX_PROPAGATION_DEPTH` | strict improvement over Phase-19 (no cycle guard at all); cycles are normal data for a generic relation, so the guard must survive release |
| B14 | Cascade soundness = enqueue-only into `deferred_hook_queue`, disjoint-allocation drain (BUG-P19-TB-1 fix) | generic hooks enqueue-only; `apply_via_raw_twin` walk unchanged; the discipline is a hook contract documented on the trait | the load-bearing invariant; Miri-TB gate covers it |
| B15 | Bubbling along `ChildOf` via `ChildOfTraversal` | `Toward<ChildOf>` (alias); any `Relationship` bubbles via `Toward<R>` | new capability, zero cost to existing |
| B16 | `Commands`/`EntityCommands` ergonomics (`add_child`, `set_parent`, …) | kept as-is (thin wrappers over `insert::<ChildOf>` / `remove::<ChildOf>`); optionally generalized later | NOT in v1 scope — ergonomics stay hierarchy-named |

Every Phase-19 behavior maps to either an unchanged kernel mechanism, a generic body that
specializes to the same code, or an explicit policy const. No behavior is lost.

---

## Generic hook pseudocode

All bodies obey the **OBS-FIRE-LOOP / F2 discipline** (copy scalars out, drop the `&`, THEN mint
`commands()` — never hold a `world`-derived `&` across a `commands()` mint; TB hazard) and the
**enqueue-only contract** (never mutate storage inline; the disjoint-allocation drain — BUG-P19-TB-1
fix — depends on it). These are the SAME bodies as today's hand-written hooks, generalized over `R`/`T`.

```rust
// ---- <R: Relationship>::on_insert  (LINK) -- monomorphizes per relation type ----
unsafe fn on_insert::<R>(view: DeferredEcsMaster, ctx: HookContext) {
    let source = ctx.entity;
    let target = match view.get_component::<R>(source) { Some(r) => r.get(), None => return };
    // drop &R borrow here (scalar `target` copied out)

    // self-ref guard: a relation to self is invalid -> reactively remove R, touch no collection
    if target == source { view.commands().entity(source).remove::<R>(); return; }
    // dangling-target guard: target must exist
    if !view.is_alive(target) { view.commands().entity(source).remove::<R>(); return; }

    // 1:1 eviction (Collection == Entity) — DEFERRED to v1.1 (W1/O3). RESERVED here so the
    // v1.1 add is a no-op on this body; `source_to_evict_before_add` defaults to `None` in v1
    // (Vec<Entity>), so this branch is dead in v1 and folds away under monomorphization:
    //   if let Some(prev) = view.get_component::<R::Target>(target)
    //                          .and_then(|t| t.collection().source_to_evict_before_add()) {
    //       view.commands().entity(prev).remove::<R>();   // prev's own on_replace unlinks it
    //   }

    // enqueue the upsert-and-push (the existing LinkCommand, now generic over R::Target):
    //   if target has R::Target -> in-place collection.add(source) (no archetype change)
    //   else -> migrate-insert R::Target::with_capacity(1) seeded with source (first-source path)
    view.commands().add(LinkCommand::<R> { target, source });
}

// ---- <R: Relationship>::on_replace  (UNLINK from old target) ----
unsafe fn on_replace::<R>(view: DeferredEcsMaster, ctx: HookContext) {
    let source = ctx.entity;
    let old_target = match view.get_component::<R>(source) { Some(r) => r.get(), None => return };
    // drop &R borrow (old_target copied out)
    view.commands().add(UnlinkCommand::<R> { target: old_target, source });
    // v1: R::Target::RETAIN_EMPTY is ALWAYS true (W1) — UnlinkCommand::apply leaves an emptied
    // collection in place (no migration). The RETAIN_EMPTY==false branch (queue removal of the
    // now-useless R::Target, the Bevy default) is RESERVED but UNIMPLEMENTED in v1 — it is a NEW
    // re-entrant edge deferred to v1.1 (W1/O3).
}

// ---- <T: RelationshipTarget>::on_replace  (CASCADE) -- only meaningful when LINKED_DESPAWN ----
unsafe fn on_replace::<T>(view: DeferredEcsMaster, ctx: HookContext) {
    if !T::LINKED_DESPAWN { /* non-cascading target: just unlink sources */ 
        for_each_source_enqueue_remove_source_relationship(view, ctx); return; }
    if cascade_suppressed() { return; }                       // CascadeSuppressGuard early-out
    // W4: cycle guard is RELEASE-active for LINKED_DESPAWN relations — cycles are normal data for
    //   a generic graph relation (unlike a tree hierarchy). A depth counter (reusing the
    //   MAX_PROPAGATION_DEPTH pattern from trigger_walk) terminates the cascade past the bound
    //   with a #[cold] warn (NOT debug_assert!). One increment per cascade level on the already-
    //   cold despawn path → ZERO cost on the 0%-gate and for non-LINKED_DESPAWN relations.
    if cascade_depth_exceeds(MAX_PROPAGATION_DEPTH) { return cascade_depth_overflow_warn(); }
    let target = ctx.entity;
    let n = match view.get_component::<T>(target) { Some(t) => t.len(), None => return };
    // INLINE path (n <= CASCADE_FANOUT_INLINE == 32): copy sources into [MaybeUninit<Entity>;32],
    //   drop &T BEFORE minting commands(), assume_init each, cmds.entity(src).despawn().
    //   (the ONE unsafe — M2 — preserved verbatim; debug-asserts n<=32 before assume_init)
    // WIDE path (n > 32): re-derive &T per turn by index, drop it before each
    //   view.commands().entity(next).despawn(); no buffer, no unsafe.
}
```

`LinkCommand::<R>` / `UnlinkCommand::<R>` are the generic forms of today's `LinkChildCommand` /
`UnlinkChildCommand`: same dangling-target guard, same in-place-push vs first-source migrate-insert
split (`migrate_entity_insert::<R::Target>`), same audited raw-archetype-id projection
(BUG-MIGRATE-TB-1). They are generic structs carrying `target`/`source`, monomorphized per `R`.

Complexity: `on_insert` O(1) amortized (push, or one migration on first source). `on_replace`
(unlink) O(k) `swap_remove` scan worst-case for `Vec` (k = collection len) — same as today. Cascade
O(n) over sources. Cache behavior: the collection is a contiguous `Vec<Entity>` (sequential read in
the cascade); the per-source despawn enqueue is a deferred command write (streaming append).
Branching: the guards (self-ref, dangling, suppress) are cold/predict-not-taken. SIMD: N/A (entity-
id-keyed, pointer-chasing despawn).

---

## Derive-macro spec

Two new `proc_macro_derive`s in `boyko_macros/lib.rs`, additive over `#[derive(Component)]`. Both
REQUIRE the type to also `#[derive(Component)]` (documented; a compile-fail test guards the misuse).

### `#[derive(Relationship)]` + `#[relationship(target = <Type>)]`

- **Attribute grammar**: `#[relationship(target = <Type>)]` — `target` (the `RelationshipTarget`
  type) is required. Optional `allow_self_referential` bare flag (sets the self-ref guard to permit
  self-links; default off).
- **Foreign-key field selection** (mirror Bevy's `relationship_field()`): tuple struct with one
  field → that field; named struct with one field → that field; named struct with multiple fields →
  the field annotated `#[relationship]` (compile error if zero or >1); unit struct → compile error.
  The field type must be `Entity`.
- **Generated**:
  ```rust
  impl Relationship for #Self {
      type Target = #Target;
      #[inline] fn get(&self) -> Entity { self.#field }
      #[inline] fn from(target: Entity) -> Self { Self { #field: target, ..Default::default() } }
      unsafe fn on_insert(v, c) { generic_relationship_on_insert::<Self>(v, c) }
      unsafe fn on_replace(v, c) { generic_relationship_on_replace::<Self>(v, c) }
  }
  ```
  Plus, **emitted into the `Component` derive's existing codegen** (so it lands in the same
  `register_hooks` / `component_id()` the `Component` derive already builds — composed, not a
  separate impl block):
  - `const HAS_HOOKS = true;` + `register_hooks { hooks.on_insert = Some(<Self as Relationship>::on_insert); hooks.on_replace = Some(<Self as Relationship>::on_replace); }`
    (reusing `ComponentHookPaths::codegen`'s assignment shape).
  - **Auto entity-remap clone metadata**: `CLONE_BEHAVIOR = CloneViaFn`, `clone_fn` = clone-via-clone,
    `map_entities_fn` remapping the foreign-key field (the generic equivalent of
    `child_of_map_entities`). Installed via the existing `install_clone_fn` / `install_map_entities_fn`
    calls the `Component` derive already emits.
  - **Auto entity-remap serialize metadata**: `SERIALIZABILITY = SerializeViaFn`, the `WireBridge`
    glue for the `Entity` field, `map_entities_fn()` load-remap, and the folded `LAYOUT_FINGERPRINT`
    (the same value the `Component` derive folds for a one-`Entity`-field struct).
- **Collision rule**: if the user also wrote `#[component(on_insert=...)]` / `#[component(on_replace=...)]`,
  the relationship owns those slots → **compile error** (a compile-fail test guards it).

### `#[derive(RelationshipTarget)]` + `#[relationship_target(source = <Type> [, linked_despawn] [, retain_empty])]`

- **Attribute grammar**: parsed in a `while !input.is_empty()` loop with `lookahead1` (mirror the
  `#[relationship_target(...)]` and existing `#[require(...)]` parse style). `source = <Type>`
  required; `linked_despawn` bare flag (sets `LINKED_DESPAWN = true`); `retain_empty` bare flag
  (sets `RETAIN_EMPTY = true`).
- **Collection field selection**: the single field (tuple-one or named-one) is the `Collection`; its
  type must implement `RelationshipSourceCollection`. The macro **enforces the field is private**
  (so user code cannot write the reverse index) — a named public field is a compile error.
- **Generated**:
  ```rust
  impl RelationshipTarget for #Self {
      type Source = #Source;
      type Collection = #FieldType;
      const LINKED_DESPAWN: bool = #linked_despawn;
      const RETAIN_EMPTY:   bool = #retain_empty;
      #[inline] fn collection(&self) -> &Self::Collection { &self.#field }
      #[inline] fn collection_mut_risky(&mut self) -> &mut Self::Collection { &mut self.#field }
      #[inline] fn from_collection_risky(c: Self::Collection) -> Self { Self { #field: c } }
      unsafe fn on_replace(v, c) { generic_relationship_target_on_replace::<Self>(v, c) }
  }
  ```
  Plus emitted into the `Component` codegen:
  - `const HAS_HOOKS = true;` + `register_hooks { hooks.on_replace = Some(<Self as RelationshipTarget>::on_replace); }`
    — **only `on_replace`**, never `on_add`/`on_insert` (B7: no spurious first-source cascade).
  - `CLONE_BEHAVIOR = Cloneability::Ignore` (reverse index rebuilt via Link commands, never copied).

The generic bodies `generic_relationship_on_insert::<R>` / `..on_replace::<R>` /
`generic_relationship_target_on_replace::<T>` live in the runtime crate (`boyko_ecs`), so the macro
emits only fully-qualified path references — no logic in the macro, matching the existing
attribute-path → hook-slot pattern.

---

## ChildOf / Children refactor change-list

`crates/boyko_ecs/src/ecs/core/hierarchy/mod.rs`:
- **Delete** the hand-written `register_hooks` bodies for both (lines 362-365, 395-397) and the
  hand-written hook wiring; **replace** with `#[derive(Component, Relationship)] #[relationship(target = Children)]`
  on `ChildOf` and `#[derive(Component, RelationshipTarget)] #[relationship_target(source = ChildOf, linked_despawn, retain_empty)]`
  on `Children`. (Because `boyko_macros` is a dev-dependency for library `src/`, the in-crate types
  use the **hand-written mirror** of the derive output — the same way `impl_self_bundle!` mirrors
  `#[derive(Bundle)]`. So in practice: implement `Relationship for ChildOf` / `RelationshipTarget for
  Children` by hand using the generic bodies, deleting the bespoke `child_of_on_*` / `children_on_replace`.)
- **Keep** the entity-remap clone metadata (`CLONE_BEHAVIOR = CloneViaFn`, `clone_fn`,
  `install_map_entities_fn(child_of_map_entities)`), the serialize metadata (`SerializeViaFn`,
  `WireBridge`, `child_of_load_map_entities`, `LAYOUT_FINGERPRINT`) — these are exactly what the
  generic `#[derive(Relationship)]` would auto-emit; the hand mirror keeps them verbatim.
- **Keep** `Children`'s `Cloneability::Ignore`, the public read-only API (`as_slice`/`len`/…), and
  `CASCADE_FANOUT_INLINE = 32`.
- **Keep** `component_id()`'s install hub for both (the C2 `install_hooks` call, clone/serialize
  installs) — unchanged.

`crates/boyko_ecs/src/ecs/core/hierarchy/commands.rs`:
- **Replace** `child_of_on_insert` / `child_of_on_replace` / `children_on_replace` bodies with calls
  into the new generic `generic_relationship_on_insert::<ChildOf>` / `..on_replace::<ChildOf>` /
  `generic_relationship_target_on_replace::<Children>`. **Generalize** `LinkChildCommand` /
  `UnlinkChildCommand` → `LinkCommand<R>` / `UnlinkCommand<R>` (or keep the named ones as
  `type LinkChildCommand = LinkCommand<ChildOf>`). The `apply` bodies (dangling guard, in-place push
  vs first-source migrate-insert, `swap_remove_entity`, RETAIN_EMPTY handling) move into the generic
  command, parameterized by `R::Target`.
- **Keep** the `CASCADE_SUPPRESS` thread-local + `CascadeSuppressGuard` (kernel-level, relation-agnostic).
- **Keep** `ClearChildrenCommand` / `DespawnWithoutChildrenCommand` (hierarchy-specific ergonomics).

`crates/boyko_ecs/src/ecs/core/component/hooks/deferred_master.rs`:
- **Rename** the existing `has_parent(entity) -> bool` (deferred_master.rs:129) to
  `is_alive(entity) -> bool` — it is ALREADY a generic liveness check (its own doc comment says
  "a generic liveness check on any entity"; it delegates to `EcsMaster::has_entity`), only the name
  is parent-specific (B3). Body unchanged. Retain a `#[doc(hidden)] #[inline] pub fn has_parent`
  thin alias forwarding to `is_alive` so the Phase-19 hand-mirror `ChildOf` call site keeps
  compiling. The generic `<R>::on_insert` dangling-target guard calls `view.is_alive(target)`.

`crates/boyko_ecs/src/ecs/core/component/observers/traversal.rs`:
- **Add** `Toward<R: Relationship>` (§Decision 5); **redefine** `ChildOfTraversal` as `Toward<ChildOf>`
  (or keep the struct, reimplement `next` via `get`). The existing `Trigger::Traversal` plumbing is unchanged.

New module `crates/boyko_ecs/src/ecs/core/relationship/` (or under `component/`):
- `mod.rs`: `Relationship`, `RelationshipTarget` traits + `LinkCommand<R>` / `UnlinkCommand<R>`.
- `collection.rs`: `RelationshipSourceCollection` trait + `impl` for `Vec<Entity>` and `Entity`.
- `generic_hooks.rs`: `generic_relationship_on_insert::<R>` / `..on_replace::<R>` /
  `generic_relationship_target_on_replace::<T>`.
- Re-export from `prelude.rs`.

`crates/boyko_macros/src/lib.rs`:
- **Add** `#[proc_macro_derive(Relationship, attributes(relationship))]` and
  `#[proc_macro_derive(RelationshipTarget, attributes(relationship_target))]`. Reuse the field-
  selection logic (clone the `relationship_field` style), the `ComponentHookPaths::codegen`
  assignment shape, and the `#[require]`-style install-triple pattern. (Note: in-crate `ChildOf`/
  `Children` won't use these derives — dev-dep cycle — but external user crates and the test crate
  will, so the derives must exist and be tested via the integration test crate.)

`crates/boyko_ecs/src/prelude.rs`: export `Relationship`, `RelationshipTarget`,
`RelationshipSourceCollection`, `Toward`.

---

## CPU test matrix (the regression + new-capability gate)

**R1 — Full Phase-19 suite as the regression gate (MUST stay green, unchanged)**:
- `phase19_hierarchy_core.rs` — all 17 tests (link both directions, unlink, reparent-atomic,
  recursive despawn 3 levels, despawn-without-children, self-ref guard, dangling guard, cascade-
  reads-current, single-outermost-drain, wide-fanout >32, clear-children, subset-remove, first-child-
  no-spurious-cascade, keep-empty, suppress-scoped-to-self).
- `phase19_hierarchy_property.rs` — `hierarchy_global_invariant_holds_after_every_op` (256 cases,
  the bidirectional `c.ChildOf==p ⟺ p.Children∋c` oracle). This is the strongest correctness gate;
  it MUST pass unchanged after the refactor.

**R2 — New `Likes`/`LikedBy` relation (proves genericity + exercises the DERIVE's auto-emit path)**:
a fresh one-to-many relation defined in the test crate via
`#[derive(Component, Relationship)] #[relationship(target = LikedBy)]` on `Likes(Entity)` and
`#[derive(Component, RelationshipTarget)] #[relationship_target(source = Likes, linked_despawn, retain_empty)]`
on `LikedBy(Vec<Entity>)`. **v1: `LikedBy` sets `retain_empty`** (W1: `RETAIN_EMPTY = true` is
MANDATORY in v1; the remove-on-empty branch is v1.1). Because the in-crate `ChildOf` uses a
hand-mirror (dev-dep cycle), the DERIVE's auto-emit path (hooks + clone-remap + serialize-remap +
fingerprint) is exercised ONLY by this external `Likes`/`LikedBy`, so these tests are the sole gate
on the derive.
- `likes_links_both_directions` — `insert::<Likes>(b)` on a → `b.LikedBy ∋ a`.
- `likes_unlink_on_remove` — `remove::<Likes>` → removed from `LikedBy` (collection retained, W1).
- `likes_retarget_atomic` — `Likes(b)` overwriting `Likes(c)` moves a between collections atomically.
- `likes_self_ref_guard` — `insert::<Likes>(self)` reactively removes `Likes`; entity stays live.
- `likes_dangling_target_guard` — `Likes(despawned)` reactively removes `Likes`; no phantom source.
- `likes_swap_remove_subset` — unlinking one source of several leaves the rest (swap_remove correctness).
- `likes_first_source_migrate` — first `Likes` into an empty target migrate-inserts `LikedBy::with_capacity(1)`.
- `likes_wide_fanout` — >32 sources exercise the cold WIDE cascade path.
- `likes_global_invariant_proptest` — the SAME bidirectional-invariant proptest, retargeted to
  `Likes`/`LikedBy` (proves the generic body upholds the invariant for an arbitrary relation).
- `likes_linked_despawn_cascade` — despawning a `LikedBy` target recursively despawns its `Likes`
  sources (proves `LINKED_DESPAWN` cascade is generic, not `ChildOf`-special).
- `no_cascade_when_linked_despawn_off` — a second relation with `linked_despawn` UNSET: despawning
  the target unlinks sources but does NOT despawn them (the non-cascading branch).
- **W3 install-probe** `likedby_registers_only_on_replace` — assert the `RelationshipTarget` derive
  set EXACTLY `hooks.on_replace` and left `on_add` / `on_insert` / `on_remove` UNSET (the B7
  spurious-first-cascade guard; mirrors `hooks_install_for_child_of_and_children`).
- **C2(b)(i) derive install-probe** `likes_derive_installs_remap_metadata` — assert the
  `#[derive(Relationship)]` on `Likes` installed `map_entities_fn` (clone-remap) + `SerializeViaFn`
  with the `WireBridge` load-remap + a NON-TRIVIAL `LAYOUT_FINGERPRINT` computed from `Likes`'s
  actual layout (NOT `ChildOf`'s hard-coded transparent/offset-0 value); mirrors
  `clone_install_for_child_of_and_children`.
- **C2(b)(ii) clone-remap behavioral** `likes_deep_clone_remaps_foreign_key` — deep-clone a `Likes`
  graph; assert the cloned `Likes` foreign key is REMAPPED to the cloned target (not verbatim, not
  dangling). A green link/unlink/cascade suite proves NOTHING about B10 without this.
- **C2(b)(iii) serialize-remap behavioral** `likes_serialize_roundtrip_remaps` — save a `Likes`
  graph, load into a fresh world, assert the foreign key is remapped to the loaded target's fresh
  `Entity` (B11 round-trip-with-remap).

**R3 — Miri-TB on the cascade (the BUG-P19-TB-1 gate, generalized)**:
- Keep `miri_phase19.rs` (all 10 tests) green under `-Zmiri-tree-borrows -Zmiri-ignore-leaks`,
  especially `miri_minimal_cascade_reentrant_push` (the canonical repro) and
  `miri_drain_panic_reentrant_disposition` (the Approach-C unwind disposition).
- **Add** `miri_likes_cascade_reentrant_push` — the same re-entrant-push surface on the generic
  `Likes`/`LikedBy` cascade (proves the disjoint-allocation drain holds for an arbitrary relation,
  not just `ChildOf`). This is the load-bearing new Miri test.

**R4 — Custom-trigger bubbling along a NON-ChildOf relation**:
- `bubble_along_likes` — define an event `E` with `type Traversal = Toward<Likes>`, `AUTO_PROPAGATE`,
  spawn a `Likes` chain a→b→c, trigger on `a`, assert the observer fires on a, b, c in hop order and
  stops at the chain end. Proves generic bubbling works on a relation other than `ChildOf`.
- `bubble_stops_on_missing_relation` — a node without `Likes` halts the walk (None hop).
- (Miri) `miri_bubble_along_likes` — TB-checks the per-hop re-derive (no `&` spans the next fire).

**R5 — Derive compile-fail tests** (`tests/relationship_compile_fail/`):
- `relationship_without_component.stderr` — `#[derive(Relationship)]` without `Component`.
- `relationship_hook_collision.stderr` — `#[relationship(...)]` + `#[component(on_insert=...)]`.
- `relationship_target_public_field.stderr` — a public collection field (privacy fence).
- `relationship_multi_field_no_annotation.stderr` — multi-field source struct with no `#[relationship]` field.
- `relationship_target_unit_struct.stderr` — unit struct as a target.
- **W2** `hook_cannot_get_collection_mut.stderr` — a hook body attempting to obtain `&mut`-into a
  `RelationshipTarget` collection through `DeferredEcsMaster` fails to compile (the view exposes no
  `&mut`-into-storage method; the structural cascade-soundness argument, regression-gated).
- `mismatched_relationship_pair.stderr` — a `Relationship` whose `type Target` does not name a
  `RelationshipTarget` with the inverse `type Source` (the trait-pair coherence guard).

**R6 — Cyclic `LINKED_DESPAWN` terminates (W4)**:
- `cyclic_linked_despawn_terminates` — build a cyclic `LINKED_DESPAWN` relation graph
  (A→B→…→A; cycles are normal data for a generic relation), despawn a node, assert the cascade
  TERMINATES gracefully — no hang, no unbounded `deferred_hook_queue` growth — via the
  release-active `MAX_PROPAGATION_DEPTH` depth-bound guard (the `#[cold]` warn path). This is the
  load-bearing W4 release-behavior test; a tree hierarchy could never produce the cycle, so
  Phase-19 never needed it.

---

## Edge cases + new-unsafe enumeration

Edge cases (each maps to a test):
- **EC1 self-reference** (`set R(self)`): generic `on_insert` guard removes R; entity stays live.
  Covered by R1 (`self_referential_*`) and the generic body. `allow_self_referential` opt-in flips it.
- **EC2 dangling target** (R points at a despawned entity): guard removes R; no phantom. R1.
- **EC3 retarget** (R(A)→R(B)): `on_replace(A)` + `on_insert(B)`, FIFO → atomic move. R1/R2.
- **EC4 first source** (target has no `R::Target` yet): migrate-insert `R::Target::with_capacity(1)`.
  R1 (`first_child_insert`), Miri (`miri_link_first_child_migrate`).
- **EC5 empty collection**: `RETAIN_EMPTY=true` (Children) keeps it; `=false` (Likes) removes the
  target component. Both branches tested (R1 keep-empty, R2 `likes_remove_on_empty`).
- **EC6 wide fanout** (>32 sources): cold WIDE path, re-derive per turn, no buffer. R1 + R2.
- **EC7 deep cycle** (A→B→…→A): NOT detected in Phase-19 (no guard). **v1 adds a RELEASE-active
  depth-bound guard for `LINKED_DESPAWN` relations** (W4): a depth counter reusing the
  `MAX_PROPAGATION_DEPTH` pattern from `trigger_walk`, terminating the cascade with a `#[cold]` warn
  (NOT `debug_assert!`) past the bound — cycles are normal data for a generic relation, so the
  guard must survive release. Depth-bound (allocation-free) is chosen over a visited-set; one
  increment per cascade level on the already-cold despawn path, ZERO cost on the 0%-gate and for
  non-`LINKED_DESPAWN` relations. Tested by R6.
- **EC8 1:1 eviction** (`Collection == Entity`, second `add`) — **DEFERRED to v1.1 (W1/O3)**. It is a
  NEW re-entrant edge (`on_insert` `try_remove`s the evicted holder's `R`), which would double v1's
  Miri-TB audit surface. The `source_to_evict_before_add` collection-trait hook stays RESERVED
  (default `None`) so the `Entity` collection + eviction path is a clean v1.1 add. NOT tested in v1.
- **EC9 duplicate relate** (same pair twice, Vec collection): adds the source twice (no dedup). This
  matches Phase-19; documented footgun; set-backed collection is the opt-in fix.
- **EC10 panic mid-cascade**: the Approach-C `catch_unwind` `[survivor][re-entrant]` re-home in
  `apply_via_raw_twin` is unchanged and relation-agnostic. Miri (`miri_drain_panic_*`).

New `unsafe` enumeration (the design adds NO new unsafe class beyond Phase-19; it relocates the
existing ones into generic bodies):
- **U1** — the inline-cascade `assume_init` over `[MaybeUninit<Entity>; 32]` (Phase-19 M2 unsafe).
  Now lives in `generic_relationship_target_on_replace::<T>`, identical invariant: `debug_assert!`
  the count `≤ CASCADE_FANOUT_INLINE` before `assume_init`; the buffer is filled then read in the
  same scope; `&T` dropped before `commands()` mint. `// SAFETY:` comment carries the invariant.
- **U2** — the audited raw archetype-id projection in the first-source migrate path
  (BUG-MIGRATE-TB-1: `addr_of!((*archetype_ptr).id).read()`). Now in `LinkCommand::<R>::apply`,
  verbatim from `insert_command.rs:74`'s audited form. Same invariant, same `// SAFETY:`.
- **U3** — `migrate_entity_insert::<R::Target>` in the first-source path. Existing audited helper,
  now generic over `R::Target` instead of hard-coded `Children`. No new invariant.
- **U4** — `HookFn` coercion: the generic `<R as Relationship>::on_insert` (an `unsafe fn`) coerced
  to a `HookFn` pointer in `register_hooks`. Same coercion the existing codegen does; no new unsafe.
No new `unsafe` arises from the generalization; every block carries the relocated `// SAFETY:`.

The **cascade soundness is STRUCTURAL, not a documented contract** (W2). It holds by construction,
not by discipline:
1. A hook's ONLY world handle is `DeferredEcsMaster`, which exposes NO `&mut`-into-storage method
   (`get_component` returns `&T` only; there is intentionally no `get_component_mut` /
   `collection_mut` on the view — confirmed at `deferred_master.rs:11-17`). A hook therefore
   cannot construct an aliasing `&mut` into a `RelationshipTarget` collection — the capability is a
   *missing method*, not a rule a programmer must remember.
2. The `*_risky` mutators (`collection_mut_risky`, `from_collection_risky`) require `&mut Self`,
   obtainable ONLY inside a `Command::apply` running under `&mut EcsMaster` — NEVER inside a hook
   (a hook holds `DeferredEcsMaster`, which has no `&mut`-into-storage path to a component). So
   monomorphization cannot mint an aliasing `&mut` from any hook body, generic or hand-written.
Therefore every relationship-maintenance hook can ONLY enqueue into `deferred_hook_queue`, and the
`apply_via_raw_twin` disjoint-allocation drain (the BUG-P19-TB-1 fix) stays sound for ANY relation
— the same structural reason it is sound for `ChildOf`. A `# Safety` doc note on the traits records
this, and a **compile-fail test (R5) asserts a hook body cannot obtain `&mut`-into a
`RelationshipTarget` collection** (turning the structural argument into a regression-gated fact).
Any `collection_mut_risky` on the trait is documented unreachable from a `DeferredEcsMaster`
context.

---

## Open questions

- **O1**: `RelationshipSourceCollection` backing — `Vec<Entity>` for v1 (matches `Children`); defer
  `SmallVec`/dense until a fanout-distribution measurement justifies inline storage. (Open.)
- **O2 (RESOLVED, W4)**: cycle guard form — depth-bound (allocation-free), RELEASE-active for
  `LINKED_DESPAWN` relations, `#[cold]` warn past `MAX_PROPAGATION_DEPTH`. Not debug-only (cycles are
  normal data for a generic relation).
- **O3 (RESOLVED, W1)**: 1:1 (`Collection == Entity`) is DEFERRED to v1.1. v1 ships ONLY
  `Vec<Entity>` (one-to-many) with `RETAIN_EMPTY = true` mandatory, so v1 introduces EXACTLY ONE new
  re-entrant surface (the generalized cascade). The `source_to_evict_before_add` collection hook and
  the `RETAIN_EMPTY = false` branch stay RESERVED for a clean v1.1 add.
- **O4**: generalize the `add_child`/`set_parent` ergonomics into `add_related::<R>`/`relate::<R>`
  in v1, or keep them hierarchy-named and add generic ergonomics later? Recommend: keep hierarchy
  ergonomics as-is in v1 (no churn), add generic `relate`/`unrelate` ergonomics in a follow-up.
  (Open.)
- **O5 (RESOLVED)**: keep `on_replace` (not Bevy-main's `on_discard`); renaming the kernel slot is a
  cross-cutting rename out of scope. Document the composite overwrite/remove/despawn semantics on the
  trait.
