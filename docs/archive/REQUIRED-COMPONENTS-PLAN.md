> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Required components `#[require(B, C)]` (task: Required) — resolved plan

Branch `ecs`, 2026-06-16. Feature 1 of three (Required → Observers/on_despawn →
Cloning, built in sequence). Produced by research → architect → 2 critics
(correctness + cross-feature/perf); this doc is the design with every CRITICAL/MAJOR
critique finding folded in as a resolved decision. It is the implementation spec.
The developer must still `graphify`-orient + read source for exact signatures.

## Goal

Declarative `#[require(B, C = ctor)]` on component `A`: inserting `A` (spawn OR
insert) auto-inserts every transitively-required component that is ABSENT, each
constructed from a registered default or capture-free ctor, firing its
`on_add`/`on_insert` hooks+observers **exactly once**, deps-before-dependent, with
**archetype-cached zero-recompute** expansion and **provably 0% cost** when a
component declares no requires.

## Core design (decisions 1–5)

**D1 — two parallel cold tables, NOT a `ComponentLayout` field** (keeps the 56 B
TRIPWIRE-2 hot record pinned; mirror `HOOKS`/`STORAGE_KIND`):
```rust
static REQUIRES_DIRECT: [OnceLock<&'static [RequiredEntry]>; MAX_COMPONENTS] = [const { OnceLock::new() }; MAX_COMPONENTS];
static REQUIRES_ALL:    [OnceLock<&'static RequiredPlan>;    MAX_COMPONENTS] = [const { OnceLock::new() }; MAX_COMPONENTS];
```

**D2 — constructor = capture-free `unsafe fn(*mut u8)`** (mirror `DropFn`;
F2-immune because it never sees `world`):
```rust
pub type RequiredCtor = unsafe fn(dst: *mut u8);
#[derive(Clone, Copy)] #[repr(C)]
pub struct RequiredEntry { pub component_id: ComponentId, pub ctor: RequiredCtor } // 16 B POD
pub struct RequiredPlan  { pub entries: &'static [RequiredEntry] }                  // transitive, DFS-ordered, deduped
```
Derive lowers `#[require(B)]` → `unsafe fn __require_ctor_B(dst){ dst.cast::<B>().write(B::default()) }`;
`#[require(C = expr)]` → `...write({ expr })` (capture-free expr only — no `Arc<dyn>`).
`#[require(B)]` needs `B: Default`; `= expr` is the no-default escape hatch.

**D3 — transitive closure computed ONCE per component, lazily, memoized in
`REQUIRES_ALL`** (DFS over `REQUIRES_DIRECT`, pulling each dependency's already-built
closure first). See the conflict rule (W1 fix) + cycle break (W2 fix) below.

**D4 — expand the archetype at the bundle-resolution funnel, cache on the Phase-8.5
slot.** Spawn: in `cold_register_bundle_archetype<B>`, effective set = `ids ∪
⋃ REQUIRES_ALL[c].entries.ids`, canonical-sort, `get_or_create_archetype(&effective)`
→ cached in the existing `Box<[OnceLock<ArchetypeId>; MAX_BUNDLE_TYPES]>`. Insert: same
union into `merged_archetype_id<B>`. The "missing-required to actually construct" list
is cached alongside `BundleColumnCache` (a `required_missing: &'static [RequiredEntry]`
+ parallel `required_pool_ids`). Warm spawn = one slot load + N_missing ctor writes.

**D5 — constructor pass fires through the EXISTING fire sites, once each, add-then-insert
order.** After the user-bundle write loop, a `const { B::HAS_REQUIRES }`-gated pass writes
each missing required component (`(entry.ctor)(pool.row_ptr_mut(row))` + `commit_units` +
`fill_ticks`), THEN the existing fire loop fires on_add/on_insert. See the C1/C2 fixes —
this is the load-bearing correction.

## CRITICAL fixes folded in (from the two critiques)

### FIX C1/X1 — the insert path does NOT fire over the full archetype (the headline bug)
Grounded: `SpawnAtCommand::apply` (spawn_at_command.rs:275,289) iterates the FULL
archetype `component_ids` → required rows fire automatically there (✓, do NOT add a
second fire). BUT `migrate_entity_insert` (migration_helpers.rs:607,614,630) fires over
`bundle_id_set = &bundle_ids[..bundle_id_count]` ONLY (gathered in `for_each_component_bytes`),
on_add additionally filtered by `bundle_added[i]`. A required component auto-inserted by
the constructor pass is NOT a bundle id → under the current loop it fires **neither on_add
nor on_insert** on the insert path (= the Phase-14b "undercounting fire sites" class:
`#[require(B)]` where B has on_add works on spawn but **silently breaks on insert**).
**Resolution:** the insert-path constructor pass MUST push each constructed (absent-in-source)
required id into the fire-iteration set with its `added` flag = true, so the existing insert
fire loop covers it. A required id already present in the source is NOT in `missing` →
not constructed, not re-fired (the "don't overwrite present B" semantic). **Mandatory
distinct tests** (spawn-path tests do NOT cover this): `insert_a_requires_b_fires_b_on_add`
AND `insert_a_requires_b_present_does_not_fire`. Recall: APPROVED-per-plan never catches an
incomplete fire-site list — only behavioral coverage of BOTH the direct AND the
Commands-deferred-apply insert path does. Verify the deferred `Commands::insert` apply site
routes through the same `migrate_entity_insert` (it does) so it inherits the fix.

### FIX C2 — the insert-path fire scratch is sized `MAX_BUNDLE_ARITY` (≤12), required ids overflow it
Grounded: `bundle_ids`/`bundle_added` are `[_; MAX_BUNDLE_ARITY]` (migration_helpers.rs:315-317),
with `debug_assert!(bundle_id_count < MAX_BUNDLE_ARITY)` at :450. A 2-component bundle that
transitively requires 15 others would write past the bound → stack buffer overrun (release)
or panic (debug). **Resolution:** the augmented fire-iteration set on the insert path must be
sized to the archetype-column bound `MAX_MIGRATION_COLUMNS` (NOT `MAX_BUNDLE_ARITY` — that is
a bundle-arity invariant; required components are an archetype-level concern with a larger
bound). Either widen the fire scratch to `MAX_MIGRATION_COLUMNS` or use a separate
required-fire scratch of that size, and reconcile/replace the `< MAX_BUNDLE_ARITY` assert
with the correct bound.

### FIX W1 — conflict-resolution rule stated precisely (drop the word "shallowest")
The rule is exactly ONE operation pair, no `inheritance_depth` integer:
- **inherited duplicates: FIRST DFS-reached ctor wins** (`if !seen { seen.insert; out.push }` keep-first);
- **a DIRECT declaration on the requiring component OVERRIDES** any inherited ctor for the
  same id (`out.iter_mut().find(id).ctor = entry.ctor`, applied after its inherited deps).
Do NOT say "shallowest" (implies a depth comparison the algorithm does not perform — it is
"first in the precedence-ordered DFS"). **Mandatory test** for the un-obvious case: two SIBLING
requires both transitively pull the same grandchild D with different ctors, neither directly
declaring D → assert the first-DFS (earlier-listed sibling's) D-ctor wins.

### FIX W2 — real cycle break, fail-loud at registration (NOT a vanishing debug_assert)
Memoization does NOT break a cycle (A→B→A): `REQUIRES_ALL[A]` is set only AFTER A's recursion
completes, so a re-entrant `build_required_plan(A)` sees `None` and recurses forever — in BOTH
debug and release. **Resolution:** thread a "currently-building" marker DISTINCT from the
memoized result (a thread-local or passed-down visited stack / per-id building-bit). On
re-entering an id already on the building stack → **panic with a named diagnostic**
(`RequiredError::Cycle`) at registration/first-expansion (a real runtime check present in
release, not a `debug_assert` that vanishes where the overflow actually crashes). Test:
`require_cycle_panics`.

### FIX W3 — pair the size assert with a `ComponentId` companion
`const _: () = assert!(size_of::<RequiredEntry>() == 16);` AND
`const _: () = assert!(size_of::<ComponentId>() == 8);` so the 16 is self-documenting, not
coincidental (guards a future `ComponentId` widening).

## Resolved open questions

- **Duplicate `#[require(B)]` on one component** → **compile error** (the macro sees both keys;
  strictly better than Bevy's runtime panic).
- **Custom ctor** → capture-free expressions / const ctors only (forced by the no-`Arc`/no-`dyn`
  fn-ptr choice). Accepted; deterministic, no heap.
- **Ctor signature** → bare `unsafe fn(*mut u8)` (leanest; no v1 ctor needs entity/tick). A
  richer `(dst, entity, tick)` kind can be added later without breaking this one.
- **Present ⇒ skip** → inserting A when B already present keeps the explicit B (no overwrite, no
  second on_add).
- **Runtime `register_required_by_id`** → **derive-only in v1** (Bevy's runtime path is where
  most of its bugs live). The `install_required` + `was_ever_archetyped` staleness gate backs a
  v1.1 runtime path; do NOT build it now.
- **Expansion site** → per-world `cold_register_bundle_archetype` (reuses the Phase-8.5 cache,
  correct re: the register-before-archetyped staleness gate). `BundleStaticInfo` stays the user's
  declared ids only (stable process-global).
- **Relationship requires** (`#[require(ChildOf)]`): a capture-free ctor cannot supply a parent
  `Entity`, so a required `ChildOf` could only point at a fixed/dummy entity (a dangling-parent
  footgun). The macro cannot reliably detect "is a relationship component", so **document this as
  unsupported/footgun** (do not hard-reject — `ChildOf` is just a `Component`); a `#[require]` of a
  relationship component is a user logic error, not an engine soundness issue. Note it in the
  derive docs.

## Cross-feature contracts (for the later features)

- **Clone (Feature 2)** must NOT route through required-expansion for the common case (it copies
  real bytes). BUT the cross-feature audit's C2 case — clone A when the source LACKS required B —
  is resolved IN FEATURE 2 (reconstruct B via `build_required_plan`, Bevy-0.17 semantics), reusing
  THIS feature's closure + missing-required diff keyed on the clone target id set. This plan
  exposes `build_required_plan` / the missing-required diff as `pub(crate)` so Feature 2 can reuse it.
- **Observers (Feature 3)**: required components fire through the standard fire sites (D5), so
  entity-targeted observers + on_despawn (built next) apply to required components for free.

## 0%-gate (sacred) — proof obligations

A `#[derive(Component)]` with no `#[require]`, spawned/inserted/queried, must be byte-identical:
(1) trait defaults `HAS_REQUIRES=false` + empty `register_required`; (2) derive emits
`install_required::<Self>` ONLY when `#[require]` present (macro-gated, like storage_install);
(3) the archetype-expansion union loop runs zero inner iterations (every `entries` empty) →
`get_or_create_archetype(ids)` with the same slice — and it is cold-path-only; (4) the hot
constructor pass is `if const { B::HAS_REQUIRES }`-gated → const-folds away entirely for a
require-free B (same mechanism as `HAS_HOOKS`/`HAS_TYPED_WRITE`); (5) the fire site is unchanged
for the require-free archetype. Validate with `gate_spawn_no_require` criterion (±2%, same-binary
A/B per the box's drift caveat) + `gate_schedule_run`/`gate_query_iter` flat.

## Soundness

No new unsafe on the read/query path. The constructor-pass write reuses the EXACT Step-5
bundle-write SAFETY invariants (pool_idx in range, row < committed, &mut archetype exclusive,
dst aligned+uninit). Capture-free ctors are F2-immune (never see `world`). The dangling-`&[u8]`
UAF class is structurally avoided (ctors write through fn-ptrs into pool slots; no bundle slice,
no two-pass collect; the user bundle's `for_each_component_bytes` completes single-pass before the
ctor pass). Diamond/double-construct prevented by the `seen` bitset + the `missing` diff. A
panicking ctor: `commit_units` runs AFTER the ctor, so an uncommitted slot is never dropped;
earlier-written components are owned by the archetype (drop on teardown) — matches existing
`for_each_component_bytes` panic discipline (catch_unwind forbidden here).

## Implementation order (developer)

1. `component_registry.rs`: `RequiredCtor`/`RequiredEntry`(+both size asserts)/`RequiredPlan`,
   the two static tables, `install_required::<C>`, `build_required_plan` (memoized DFS + the W2
   cycle-break visited stack + W1 conflict rule), `get_required_plan`. Mirror the `HOOKS` block.
2. `component.rs`: `const HAS_REQUIRES = false`, `fn register_required(_) {}`, `RequiredBuilder`.
3. `boyko_macros/src/lib.rs`: parse `#[require(B, C = expr, D(7))]` (reject dup-same-id at compile
   time), emit `HAS_REQUIRES`, `register_required`, the free `__require_ctor_*` fns, and the gated
   `install_required::<Self>(raw)` in `component_id()`.
4. Archetype expansion in `cold_register_bundle_archetype` + `merged_archetype_id` (stack scratch,
   no heap).
5. `BundleColumnCache`/`ResolvedBundle`: cache `required_missing` + `required_pool_ids`.
6. `spawn_at_command.rs`: the `const`-gated constructor pass after Step 5 (spawn fire already covers).
7. `migrate_entity_insert`: the constructor pass + the C1 fire-set augmentation + the C2 scratch
   sizing.
8. Tests + benches (below).

## Tests (mandatory)

`require_single`, `require_recursive`, `require_diamond` (D once, first-DFS winner),
`require_conflict_direct_wins`, `require_conflict_sibling_first_dfs` (W1), `require_custom_ctor`
(no-Default type via expr), `require_on_add_fires_once` (hook+observer, add-then-insert order),
`require_does_not_overwrite_present`, **`insert_a_requires_b_fires_b_on_add`** (C1, distinct from
spawn), **`insert_a_requires_b_present_does_not_fire`** (C1), `insert_via_commands_deferred_fires`
(C1 deferred-apply site), `require_cycle_panics` (W2), property test (random acyclic DAG →
order-independent canonical archetype, each required id once, each on_add once). Benches:
`gate_spawn_no_require` (±2%), `gate_schedule_run`, `require_spawn_warm`, `require_spawn_all_supplied`.
Miri-TB on the constructor pass. clippy clean. gnu-1.96 (`+stable-x86_64-pc-windows-gnu`, pkg `boyko-ecs`).
