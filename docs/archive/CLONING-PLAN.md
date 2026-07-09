> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Entity cloning / EntityCloner (Feature 3 of 3) — resolved plan

Branch `ecs`, 2026-06-16. A per-entity clone primitive: materialize a new entity (or
write into an existing target) carrying clones of a source's components.
Reflection-free, fn-ptr-driven, relationship-aware (deep/shallow over ChildOf). Built
last (depends on Feature-1 required-reconstruction + Feature-2 observer fire infra,
both shipped). Produced by research → architect → 2 critics; this folds the
CRITICAL/MAJOR critique findings in as resolved decisions. Full design at
`D:\tmp\p1_wf\design_cloning.md`. Developer `graphify`-orients + reads source.

## Core design (decisions; see the design doc for full bodies)

- **D1 per-ComponentId clone metadata in a parallel cold table** `CLONE: [OnceLock<
  CloneInfo>; MAX_COMPONENTS]` (mirror HOOKS/STORAGE_KIND; keeps ComponentLayout at
  56 B). `CloneInfo { clone_fn: Option<CloneFn>, cloneability: Cloneability }`.
  `CloneFn = unsafe fn(*const u8, *mut u8)` (bare fn-ptr, no Box<dyn>/Arc).
- **D2 auto-from-Clone WITHOUT a `Clone` bound on `Component`** (existing non-Clone
  components must keep compiling). Two defaulted trait items: `const CLONE_BEHAVIOR:
  Cloneability = Ignore` + `fn clone_fn() -> Option<CloneFn> { None }`. The derive
  detects Copy/Clone/Entity-field and overrides; `#[component(no_clone)]` opts out;
  `#[component(clone = expr)]` custom (capture-free free fn). Non-Clone = `Ignore`
  (clone_fn None) → cloner policy skips (opt-out) or rejects (strict).
- **D3 batch fast path** = `Cloneability::TriviallyCopyable` (Copy + NO Entity field)
  → whole-column `copy_nonoverlapping` (the -45% Bevy win, reuses `write_at`). A
  Copy-WITH-Entity component is classified `CloneViaFn` (NOT Trivial) so the entity
  remap (D5) can run. **(O2/Open-Q1 fix below collapses the dual fn-ptr path.)**
- **D4 EntityCloner + typestate builder** (`OptOut`/`OptIn`), `BitSet512` filter (no
  HashMap), `linked`/`fire_hooks`/`strict`/`preserve_ticks`. Cloner is pure config
  (Send+Sync, reusable); execution single-threaded on `&mut EcsMaster`.
- **D5 ChildOf-only relationship remap in v1** (boyko's hierarchy is hardcoded; a
  general per-component `map_entities_fn` is over-engineering). Shallow = ChildOf
  copied verbatim (clone is a sibling), Children always denied. Deep = clone the
  subtree, remap each ChildOf to the cloned parent via a `MAP_ENTITIES` table
  (installed only for ChildOf), rebuild Children via the canonical `LinkChildCommand`.
- **D6 clone FIRES on_add/on_insert by default** (toggleable `fire_hooks(false)` for
  bulk), reusing the existing ArchetypeFlags-gated fire sites (0% when the archetype
  has no hooks). `clone_components`-into-overlap fires on_replace+on_insert (reused).

## CRITICAL/MAJOR fixes folded in (the critique blockers)

### FIX C2 (CRITICAL cross-feature) — clone RECONSTRUCTS a source-missing required component (replaces Decision 7's "never re-expand, period")
Decision 7 said clone copies real bytes + never re-expands requires. That is correct
for the COMMON case (source has all its required components → copy their real,
possibly-mutated bytes, do NOT re-Default). BUT cloning A when the source LACKS A's
required B (B removed at runtime, or `#[require]` added after the entity was built, or
the filter DENIED B) would land the clone in an archetype where A is present but B is
not — **violating the require-invariant that an on_add observer for A may rely on**
(Bevy resolved this in 0.17, issue #19324). **Resolution:** the clone's target archetype
= the require-CLOSURE of (source ∩ filter). For each required id that is PRESENT in the
target set, copy its real source bytes (no re-default — Decision 7 preserved for cloned
components). For each required id ABSENT from the target set (source lacked it, or the
filter denied it but a cloned component requires it), **reconstruct it via Feature-1's
missing-required ctor pass** (reuse `build_required_plan` + the missing-required diff,
which Feature 1 exposed `pub(crate)`), keyed on the clone target id set. A cloned
component's required dependency is therefore ALWAYS present (the deny of a required
component is overridden + debug-logged — Bevy 0.17 "allowed components also allow their
required"). This makes clone consistent with insert: both preserve the invariant. Tests:
`clone_source_lacks_required_b_reconstructs_b`, `clone_present_required_b_keeps_real_value`
(no re-default), `clone_filter_denies_required_b_reconstructed_with_warning`. This is the
deepest cross-feature coupling — flag it for the dev/review/tester.

### FIX W5 (CRITICAL soundness) — the rollback guard must NOT touch `entity_master` in `Drop` (the F2 class)
The design's `CloneRowGuard::Drop` rolled back BOTH the archetype row AND the reserved
entity in `entity_master` — but reaching `entity_master` from `Drop` requires a cached
`NonNull<EcsMaster>` written in `Drop` while the unwinding frame holds `&mut world` =
the F2/9.3c Tree-Borrows class. **Resolution — reorder so `Drop` only touches the
archetype row (sound via the raw slab archetype ptr), never `entity_master`:** reserve
the `EntityId` but do NOT commit the entity→inland mapping in `entity_master` until
AFTER materialization fully succeeds (the LAST step, after the guard disarms). Then a
panic during materialization: the guard (holding only `target_archetype_ptr + new_row +
committed_count`, all archetype-local, interior-mutable slab provenance) drops the
`0..committed` cloned components via each pool's `drop_at` and uncommits the row — no
`entity_master` mutation needed (the entity was never mapped). The reserved EntityId is
either left to the entity-master's normal reuse or freed by the caller in plain control
flow on the error path (NOT in `Drop`). No cached world pointer written in `Drop`.
Miri-TB is the oracle (mandatory test). State the exact mechanism; do NOT `catch_unwind`
on this path (forbidden) — the guard's `Drop` + the deferred entity-mapping-commit gives
strong exception safety without it.

### FIX W6 — deep-clone snapshots `Children` into an owned buffer before any spawn (dangling-slice class)
The design seeded the deep-clone worklist from `Children::as_slice()` (a borrow into the
source's Children pool) and then SPAWNED clones (structural pushes that can reallocate
pools) — the recurring Phase-11/14b/19 dangling-`&[u8]`/slice UAF class at the subtree
level. **Resolution:** snapshot each node's children `EntityId`s into the owned worklist
`Vec<(Entity, Entity)>` BY VALUE before any structural push; never hold the
`as_slice()` borrow across a spawn. Re-resolve `source_archetype_ptr` per node (the
`apply_via_raw_twin` "walk a stack-local copy / re-resolve, don't cache across structural
ops" lesson). Per-node source bytes are read single-pass inside that node's
materialization, before its push.

### FIX W4 — `EntityCloneMap` uses single-param `SparseMap<Entity>` (keyed by usize)
boyko's `SparseMap` is `SparseMap<U>` keyed by `usize` (single type param), not the
two-param `SparseMap<usize, Entity>` the design sketched. Use `SparseMap<Entity>` keyed
by `EntityId.0`. (`Entity: Copy`, so its `swap_remove`/`Clone`-on-remove is cheap — no
W8-class issue here.) For small subtrees a stack `Vec<(Entity,Entity)>` avoids the alloc
(the `CASCADE_FANOUT_INLINE` threshold pattern).

### FIX O2 / Open-Q1 — collapse the dual fn-ptr: `TriviallyCopyable` → `clone_fn = None`, batch-by-column only
The size-agnostic `CloneFn(*const u8, *mut u8)` forced a monomorphized
`memcpy_clone_typed::<C>` purely to capture `size_of::<C>()`, but the whole-column batch
path is size-driven by the pool layout and never calls the fn-ptr. **Resolution:** a
`TriviallyCopyable` component installs `clone_fn = None` (the batch-by-column
`copy_nonoverlapping(src_row, dst_row, stride)` drives it from the pool layout); the
fn-ptr (`clone_via_clone::<C>`) is `Some` ONLY for `CloneViaFn`. The `Cloneability` enum
alone drives the materialization branch (batch vs fn-ptr); no redundant dead fn-ptr for
Copy types. Removes a monomorphized-fn family + the size-agnostic-signature footgun.

### FIX W7 (state it) — `clone_via_clone` cannot reach world state
`clone_via_clone` (and the batch path) receive ONLY `*const u8`/`*mut u8` — no
`DeferredEcsMaster`/world view. So arbitrary user `Clone::clone` code, even though it
runs while `&mut Archetype`/`&mut pool` reborrows are live in the materialization loop,
cannot reach world state and cannot create the F2 protected-tag conflict. State this in
the SAFETY comment (closes the audit item).

## Resolved open questions

- **`clone_components` cross-archetype** (target gains new components) → v1 supports
  clone-and-spawn + clone_components into an OVERLAPPING-or-superset target via the
  existing migrate/overlap-replace path; a target needing genuinely new columns routes
  through `migrate_entity_insert` with clone-sourced bytes (small adapter). Confirm scope
  with the reviewer; defer the full cross-archetype adapter if it balloons.
- **`move_components`** (clone-then-remove-from-source) → DEFER to v1.1 (a thin
  composition; clone-and-spawn + clone_components cover the headline cases).
- **Deep-clone of non-ChildOf entity refs** → OUT of v1 (D5 boundary; the `MAP_ENTITIES`
  table is laid for a future general derive). Documented.
- **`preserve_ticks` default** = reset (a clone is "added now"; `Added`/`Changed` fire
  the frame it's cloned).
- **opt-out skip of an `Ignore` component** → debug-log/`debug_assert` so the
  "missing component" surprise is diagnosable (not silent).

## 0%-gate (sacred)

`ComponentLayout` unchanged (TRIPWIRE 2); `CLONE`/`MAP_ENTITIES` are separate cold
tables read ONLY from `clone/` (grep-proof: `get_clone_fn`/`get_map_entities_fn` callers
⊆ `clone/`); the derive's `install_clone_fn::<Self>` is one cold `OnceLock::set` per type
at registration (not per-frame); `Component` widening is compile-time (const + defaulted
method); materialization reachable only via the explicit `clone_*` API. A program that
never clones is byte-identical (registration-time-only delta). Validate: spawn/query/
schedule benches with the feature compiled in = "no change detected".

## Soundness (Miri-TB the oracle — new unsafe + the rollback guard)

S1 CloneFn provenance/align/non-overlap (established at the single call site via
`unit_ptr`/`row_ptr` contracts). S2 `clone_via_clone` `&C` (live/aligned/init, no &mut
alias, single-threaded). S3 `dst.write` (no drop of uninit). **S4 single-pass dangling-
slice avoidance** (consume src→dst inside the per-component iteration; deep clone
re-resolves per node + snapshots children — W6). **S5 the rollback guard (W5): Drop
touches only the archetype row via the raw slab ptr; the entity→inland mapping is
committed only after success, so Drop never touches `entity_master` / never caches a
world pointer.** S6 fire sites mint `world_ptr` only after all `&mut Archetype` reborrows
drop (F2). Run Miri-TB on: shallow clone, owning-component clone (drop-count-exact),
deep subtree, the panic-mid-row rollback (drop-count-exact + no leak + target not
half-alive), the fire path.

## Build order (developer; the design's 9-step plan)

1. Registry tables + glue (`CloneInfo`/`Cloneability`/`CloneFn`/`MapEntitiesFn`,
   `CLONE`/`MAP_ENTITIES`, install/get, `clone_via_clone::<C>`; NO `memcpy_clone_typed`
   per O2 — batch-by-column instead).
2. `Component` trait widening (`CLONE_BEHAVIOR` + `clone_fn()`, defaulted).
3. Derive: detect Copy/Clone/Entity-field, emit overrides + ungated `install_clone_fn`;
   parse `#[component(no_clone)]`/`#[component(clone=expr)]`; update the hand-written
   ChildOf/Children impls + a coverage test (mirror `hooks_install_for_child_of...`).
4. `EntityCloner` + typestate builder (`BitSet512` filter).
5. Materialization (Algorithm A): filtered id set → **require-closure (C2)** → target
   archetype → row push → single-pass per-component clone (batch vs fn-ptr) →
   missing-required reconstruct (C2) → gated fire; the CloneRowGuard (W5).
6. Direct API (`clone_and_spawn`/`_with`/`clone_components`), drain after.
7. Deep clone (Algorithm B): `EntityCloneMap` (W4), worklist (W6 snapshot), ChildOf
   remap, `LinkChildCommand` rebuild; install ChildOf `map_entities_fn`.
8. Deferred `Commands::clone_and_spawn` (entity reservation + apply-window).
9. Tests + benches + Miri-TB.

## Tests (mandatory)

Shallow all-Copy (same archetype, equal values); owning `Name(String)` (deep copy,
drop-count-exact, no double-free); Copy fast path (a Copy type whose Clone impl panics →
proves memcpy, not clone, used); allow/deny filter; strict panic vs skip; shallow ChildOf
(clone shares parent, no Children); **deep subtree** (children are clones, ChildOf
remapped, Children consistent, external parent verbatim, diamond dedup); fire on_add (on
vs off); **C2: clone reconstructs source-missing required B; keeps present required B's
real value; filter-denied required B reconstructed+warned**; clone does NOT re-default a
present cloned component (Decision 7 preserved); `clone_components` overlap fires
on_replace+on_insert; **panic-mid-row rollback** (drop-count-exact, no leak, target not
half-alive, entity_master untouched). Property: random archetype shallow-clone equality;
random tree deep-clone isomorphism + ChildOf remap. **Miri-TB** on the clone/rollback/
deep/fire paths. 0%-gate bench. gnu-1.96; clippy clean.
