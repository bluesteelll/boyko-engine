# Phase 15 Plan — Explicit System Ordering & Schedule Sets (Completing the Phase 9 Scaffold)

**Status:** Architecture plan, pre-implementation. **Branch:** `ecs`. **Author role:** lead architect.
**Premise (verified against code):** ~70% of Phase 15 already exists and is tested. This plan completes "Wave 5 Step 14", it does not redesign the proven executor or the conflict-vs-ordering interaction.

---

## §1 Scope — what is done vs this phase's delta

### 1.1 Already present and TESTED (do NOT rebuild)

| Component | Location | Evidence it works |
|---|---|---|
| `OrderingEdge` enum (`Before`/`After`/`ChainConsecutive`/`InSet`) | `ordering.rs:54-72` | `as_dag_edge_directions`, `in_set_contributes_no_edge` tests |
| `SystemKey` opaque handle | `ordering.rs:31-33` | used throughout builder |
| `SystemConfig::before/after/chain` | `system_config.rs:65-94` | push `OrderingEdge` onto descriptor |
| `SystemConfig::in_set` (records membership + mirrors to `set_members`) | `system_config.rs:103-114` | edge NOT expanded — gap |
| `SystemSet` trait + `SystemSetId`, `set_id_of` interning | `system_set.rs:31-53`, `schedule_builder.rs:117-123` | `system_set_id_equality_and_hash` |
| `#[derive(SystemSet)]` **unit-struct only** | `boyko_macros/src/lib.rs:1105-1199` | `derive_system_set_smoke.rs` (3 tests) |
| Tarjan SCC cycle detection | `schedule_builder.rs:374-456` | `tarjan_detects_three_cycle`, `cycle_in_before_after_panics` |
| Kahn topo sort (FIFO-stable) | `schedule_builder.rs:471-498` | `topological_sort_respects_before`, `kahn_sort_basic` |
| Edge dedup (`HashSet<(u16,u16)>`) | `schedule_builder.rs:258-267` | guards `pred_count` inflation |
| `ConflictGraph::build` (access conflicts + ordering-edge ⇒ conflict bit) | `conflict_graph.rs:97-176` | `pred_count_matches_in_degree` |
| Executor two-gate dispatch (`pred_remaining` + `conflict_bits`) | `schedule.rs:444-485` | `conflicting_systems_serialize` |
| Apply-window successor decrement (deferred-command visibility) | `schedule.rs:350-372` | `apply_window_sees_body_writes` |

### 1.2 This phase's delta (the 6 missing pieces + 2 cross-cuts)

| # | Deliverable | Core gap | Lands in |
|---|---|---|---|
| D1 | **Set-membership expansion** (`InSet` → pairwise edges) | `as_dag_edge` returns `None` for `InSet` (`ordering.rs:88`); `set_members` discarded (`schedule_builder.rs:159`) | new `expand_set_edges` fn, called in `build` between Step 2 and Step 3 |
| D2 | **Set-level ordering API** (system→set, set→set) | `before`/`after` take `SystemKey` only (`system_config.rs:65`); no `configure_set` | new `OrderTarget` enum + `OrderingEdge` variants + `ConfigureSet` builder handle |
| D3 | **Set hierarchy** (`set in_set set`) + transitive flatten | no representation | `set_parents` map + transitive-closure pass |
| D4 | **`#[derive(SystemSet)]` ENUM support** | macro rejects enums (`lib.rs:1173`) | extend macro; needs per-variant `SystemSetId` keying (D6 changes `set_id_of`) |
| D5 | **Diagnostics / missing-target** | foreign/stale `SystemKey` silently mis-indexes; `build` panics, no `Result` | `ScheduleBuildError` enum + `try_build`; keep `build` as panicking wrapper |
| D6 | **Set identity keying** (enables D4 enums) | `set_id_of` keys `TypeId` alone (`schedule_builder.rs:117`) | key on `(TypeId, u32 discriminant)` |
| C1 | Cycle-through-sets | confirm expanded edges feed Tarjan | no new code if D1 ordering is correct |
| C2 | Determinism | confirm Kahn FIFO survives expansion | deterministic iteration order in D1/D3 |
| D7 | **Sync-points** | `insert_sync_points` no-op (`schedule_builder.rs:355-360`) | CONFIRM stays no-op; document (§7) |

**Hard boundary (re-confirmed from code):** The per-frame path is `Schedule::run` (`schedule.rs:116`) → `executor_main_loop` (`:233`) → `try_dispatch_ready` (`:425`). It reads only `pred_remaining[i]` (`:451`) and `conflict_bits[i]` (`:459`) — both materialized once in `ConflictGraph::build`. **Every D1-D7 change executes inside `ScheduleBuilder::build` or earlier.** Zero new bytes in `run`. The Phase 9 "50 systems 1.72× vs Bevy" bench is preserved by construction (§8.3 verifies).

---

## §2 Set-membership expansion algorithm (D1)

### 2.1 The model (Bevy "flatten", adapted)

Research §1.5 step 4 and §4.4 establish the canonical operation: remove set nodes from the ordering DAG and replicate their edges to members. boyko's variant works directly on `SystemKey` pairs (no graph node type for sets), feeding the **existing** `dag_edges_keys` vec (`schedule_builder.rs:201`). Three edge sources expand to system→system pairs:

| Source ordering relation | Expansion | Edge count |
|---|---|---|
| `System(X) before Set(S)`, members(S)={s₁..sₖ} | `{X → sᵢ}` | k |
| `Set(S) before System(Y)` | `{sᵢ → Y}` | k |
| `Set(S) before Set(T)`, members(T)={t₁..tₘ} | `{sᵢ → tⱼ}` | k·m |

`members(S)` is the **transitive leaf membership** computed in D3 (§4) — every system in S directly or via nested sub-sets. `InSet(a, S)` itself contributes NO direct edge (it only populates `members(S)`); ordering arises solely from set-level relations. This matches Bevy: membership defines the group; `configure_sets`/`before` define order.

### 2.2 Where it runs in `build` (exact insertion point)

Current `build` (`schedule_builder.rs:151-304`):
```
Step 1 (166-176): initialize systems, refresh is_exclusive
Step 2 (181-184): capture names
        (187-196): cap debug_asserts
Step 3 (200-204): collect dag_edges_keys via as_dag_edge   ← MODIFY
Step 4 (209-220): Tarjan SCC                                ← feeds on expanded edges (C1)
Step 5 (225):     Kahn topo sort                            ← feeds on expanded edges (C2)
...
```

**New ordering (insert D3 + D1 between Step 2 and Step 3):**
```
Step 2.5 (NEW — D3): build set hierarchy, compute transitive members(S) for every set,
                     detect hierarchy cycle (→ ScheduleBuildError::SetHierarchyCycle)
Step 2.6 (NEW — D1): expand_set_edges → Vec<(SystemKey, SystemKey)> from
                     (set_ordering_edges, transitive_members)
Step 3   (MODIFY):   dag_edges_keys = [direct before/after/chain edges]
                                    ++ [expanded set edges]
Step 4   (UNCHANGED): Tarjan over the COMBINED edge list  ← cycle-through-sets caught here (C1)
Step 5   (UNCHANGED): Kahn over the COMBINED edge list
```

The pre-destructure at `schedule_builder.rs:156-161` changes: `sets: _sets` → `sets` and `set_members: _set_members` → `set_members` (both now consumed, ending the `_`-discard).

### 2.3 Algorithm (pseudo-code, build-time only)

```rust
// Step 2.6. Inputs:
//   set_ordering: Vec<SetOrderEdge>   // collected by configure_set / before_set (D2)
//   members: &HashMap<SystemSetId, Vec<SystemKey>>  // transitive (D3 output), deterministic order
// Output: Vec<(SystemKey, SystemKey)> appended to dag_edges_keys.
//
// Complexity: O(E_set · k_max²) worst case where E_set = #set-level edges,
//   k_max = largest transitive membership. Bounded in §2.5.
fn expand_set_edges(
    set_ordering: &[SetOrderEdge],
    members: &HashMap<SystemSetId, Vec<SystemKey>>,
) -> Result<Vec<(SystemKey, SystemKey)>, ScheduleBuildError> {
    let mut out = Vec::new();
    for e in set_ordering {
        match *e {
            // System X before Set S  →  X → each member of S
            SetOrderEdge::SystemBeforeSet(x, s) => {
                let m = members.get(&s).ok_or(ScheduleBuildError::EmptyOrUnknownSet(s))?;
                for &sys in m { out.push((x, sys)); }
            }
            SetOrderEdge::SystemAfterSet(x, s) => {
                let m = members.get(&s).ok_or(ScheduleBuildError::EmptyOrUnknownSet(s))?;
                for &sys in m { out.push((sys, x)); } // each member before X
            }
            // Set S before Set T  →  cartesian product
            SetOrderEdge::SetBeforeSet(s, t) => {
                let ms = members.get(&s).ok_or(..)?;
                let mt = members.get(&t).ok_or(..)?;
                // SetsHaveOrderButIntersect (research §1.9): a system in both S and T
                // would yield a self-edge sys→sys = trivial cycle. Detect early with
                // a precise message rather than letting Tarjan report an opaque SCC.
                if ms.iter().any(|a| mt.contains(a)) {
                    return Err(ScheduleBuildError::SetsOrderedButIntersect(s, t));
                }
                for &a in ms { for &b in mt { out.push((a, b)); } }
            }
        }
    }
    Ok(out)
}
```

`SetOrderEdge` and `members` iterate in a **deterministic order** (§8.4): `set_ordering` is a `Vec` in declaration order; `members(S)` is sorted by `SystemKey.0` at the end of D3's transitive pass. This makes `out` byte-identical across runs → the downstream dedup + Kahn produce a stable schedule (C2).

### 2.4 Why no separate set-node graph (vs Bevy's `DiGraph`)

Bevy carries sets as first-class graph nodes (research §1.3 `GraphInfo`) then flattens. boyko's `SystemKey`-pair representation skips the node abstraction: sets never enter Tarjan/Kahn as nodes, only their **expanded system edges** do. Justification:
- **Zero new graph type, zero new alloc shape** — reuses the proven `Vec<(SystemKey, SystemKey)>` → Tarjan → Kahn pipeline verbatim. Principle #5 (minimum allocations): no `DiGraph`/`IndexMap` dependency (Bevy pulls these in).
- **Trade-off:** a set-level cycle (`S before T, T before S`) is detected at the *expanded* level (SCCs of systems), so the diagnostic must reconstruct "which sets" from the system cycle (§6.3). Acceptable: the message can still name the sets via a reverse `system→sets` lookup. We pay a slightly more complex error message to avoid an entire graph-node subsystem.

### 2.5 Edge-count blowup — bound and acceptance

Pairwise `Set(S) before Set(T)` is O(k·m). Worst case: two sets each containing all N systems, ordered → N² edges. With `MAX_SYSTEMS_PER_SCHEDULE = 1024` (`schedule_builder.rs:49`), the absolute ceiling is ~1M edges → the dedup `HashSet<(u16,u16)>` and Tarjan adjacency lists would spike build memory to ~8 MB transiently.

**Bound (debug_assert + documented soft cap):**
- Pre-dedup edge count is capped: `debug_assert!(expanded.len() + direct.len() <= MAX_EXPANDED_EDGES)` where `MAX_EXPANDED_EDGES = 1 << 20` (1M). This is a build-time sanity rail, not a hot-path cost.
- **Acceptance:** build is one-shot (research §6.1 confirms all engines resolve ordering at build, never per-frame). 1M edges through Tarjan (O(V+E)) + Kahn (O(V+E)) + dedup (O(E)) is < ~50 ms even at the ceiling — irrelevant against a per-frame budget of microseconds. Realistic schedules have k,m ≤ ~30; k·m ≤ ~900 edges per set-pair, trivial.
- **Mitigation already free:** the existing dedup (`schedule_builder.rs:258-267`) collapses the common case where a system in S is also reachable through multiple set relations. Expansion is the *canonical* duplicate source (research §4.3) and the dedup was built anticipating it.

### 2.6 Cache behavior

Build-time only; not hot. Expansion writes a `Vec` sequentially (streaming append), Tarjan/Kahn touch adjacency lists with the same locality profile they have today. No D-cache regression on `run` (nothing in `run` changes). I-cache: `expand_set_edges` is `#[inline(never)]`-eligible (cold, build-only) so it does not bloat the `build` instruction footprint that matters — but `build` is itself cold, so we leave inlining to the compiler (principle #8: no blind annotation).

---

## §3 Set-level ordering API (D2)

### 3.1 Target type — generalize to `OrderTarget`

Per research §4.5 open-Q2, choose the **enum target** over discrete `BeforeSet`/`SetBeforeSet` variants. Rationale: one target type keeps `SystemConfig::before/after` signatures uniform and avoids combinatorial method explosion (`before`, `before_set`, `set_before`, `set_before_set`). The existing `OrderingEdge` keeps its `SystemKey`-pair variants for the hot diagnostic path (they name the user's exact call); set relations get a sibling enum collected separately.

```rust
// ordering.rs — NEW
/// What an ordering constraint points at: a single system (by handle) or a set.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum OrderTarget {
    System(SystemKey),
    Set(SystemSetId),
}

/// Set-level ordering relation, collected on the BUILDER (not per-descriptor),
/// because a set has no single descriptor to own it. Expanded by
/// `expand_set_edges` (§2.3) at build.
#[derive(Clone, Copy, Debug)]
pub(crate) enum SetOrderEdge {
    SystemBeforeSet(SystemKey, SystemSetId),
    SystemAfterSet(SystemKey, SystemSetId),
    SetBeforeSet(SystemSetId, SystemSetId),  // covers configure_set(S).before(T)
}
```

`OrderingEdge` (system↔system) is unchanged — `as_dag_edge` (`ordering.rs:83`) still handles `Before`/`After`/`ChainConsecutive`; `InSet` still returns `None`. System↔set and set↔set never flow through `as_dag_edge`; they flow through `expand_set_edges`.

### 3.2 `SystemConfig` additions (system-relative-to-set)

```rust
// system_config.rs — NEW methods on SystemConfig<'a>
impl<'a> SystemConfig<'a> {
    /// This system runs before every (current + nested) member of `S`.
    /// Recorded as a builder-level SetOrderEdge; expanded at build.
    #[inline]
    pub fn before_set<S: SystemSet>(self) -> Self {
        let set_id = self.builder.set_id_of_typed::<S>();   // D6 keying
        self.builder.set_ordering.push(
            SetOrderEdge::SystemBeforeSet(self.key, set_id));
        self
    }

    /// This system runs after every member of `S`.
    #[inline]
    pub fn after_set<S: SystemSet>(self) -> Self {
        let set_id = self.builder.set_id_of_typed::<S>();
        self.builder.set_ordering.push(
            SetOrderEdge::SystemAfterSet(self.key, set_id));
        self
    }
}
```

`set_ordering: Vec<SetOrderEdge>` is a **new field on `ScheduleBuilder`** (collected during chaining, consumed by `expand_set_edges`). `set_id_of_typed::<S>()` is the D6 replacement for `set_id_of(TypeId)` — it derives the `(TypeId, discriminant)` key from the marker type (for unit-struct sets the discriminant is 0; enum variants supply their own — see §5.3).

### 3.3 `configure_set` builder (set-relative-to-set + set hierarchy)

Per research §4.5 open-Q6, prefer a **dedicated `ConfigureSet` handle** returned by `ScheduleBuilder::configure_set::<S>()` — it mirrors Bevy's `configure_sets` mental model and keeps set configuration syntactically distinct from system registration (a set is not a system).

```rust
// schedule_builder.rs — NEW
impl ScheduleBuilder {
    /// Begin configuring set `S` — order it relative to other sets, or nest
    /// it inside a parent set. Interns `S` so an empty set still gets an id
    /// (lets `before_set::<S>()` resolve even if no system joined S yet —
    /// though that case warns at build, §6.4).
    pub fn configure_set<S: SystemSet>(&mut self) -> ConfigureSet<'_> {
        let set_id = self.set_id_of_typed::<S>();
        ConfigureSet { builder: self, set_id }
    }
}

/// Fluent handle for set-level ordering + hierarchy (Bevy `configure_sets`).
pub struct ConfigureSet<'a> {
    builder: &'a mut ScheduleBuilder,
    set_id: SystemSetId,
}

impl<'a> ConfigureSet<'a> {
    /// `self_set` runs before every member of `T`.
    #[inline]
    pub fn before<T: SystemSet>(self) -> Self {
        let t = self.builder.set_id_of_typed::<T>();
        self.builder.set_ordering.push(SetOrderEdge::SetBeforeSet(self.set_id, t));
        self
    }
    /// `self_set` runs after every member of `T` (recorded as T-before-self).
    #[inline]
    pub fn after<T: SystemSet>(self) -> Self {
        let t = self.builder.set_id_of_typed::<T>();
        self.builder.set_ordering.push(SetOrderEdge::SetBeforeSet(t, self.set_id));
        self
    }
    /// Nest `self_set` inside `P`: every member of self_set transitively joins P (D3).
    #[inline]
    pub fn in_set<P: SystemSet>(self) -> Self {
        let p = self.builder.set_id_of_typed::<P>();
        self.builder.set_parents.entry(self.set_id).or_default().push(p);
        self
    }
}
```

### 3.4 Builder field additions (summary)

```rust
// ScheduleBuilder — three new fields (all build-time, dropped after build):
set_ordering:  Vec<SetOrderEdge>,                 // §3.2/§3.3 — set-level edges
set_parents:   HashMap<SystemSetId, Vec<SystemSetId>>, // §4 — hierarchy (child → parents)
set_names:     HashMap<SystemSetId, &'static str>,// §6.3 — for cycle/diag messages (type_name)
// Existing: descriptors, sets (TypeId→id), set_members (id → direct members)
```

`set_names` is populated in `set_id_of_typed` via `core::any::type_name::<S>()` (folds to a `&'static str` literal — research §1.4-style, zero runtime cost) so diagnostics can name sets, not just systems.

### 3.5 Ergonomics decision: keep handle-based + add set-based (BOTH)

Per research §4.5 / the task's explicit ask: **keep `SystemKey`-handle targeting for system→system, add `SystemSet`-type targeting for everything set-related.** Justification (principle #1, #10):
- Handle-based system targeting (`before(key)`) is zero-`dyn`, unambiguous, and already tested. Removing it would be a regression.
- Set-based targeting (`before_set::<S>()`, `configure_set::<S>()`) is the ergonomic layer for cross-module ordering. It is `TypeId`-cheap (D6 key), no `dyn SystemSet`, no `Box`. We deliberately do NOT add Bevy's bare-system-as-target (`before(some_fn)`) — it reintroduces the closure-`TypeId` / ">1 instance" ambiguity that `SystemConfig`'s own doc (`system_config.rs:11-22`) records as the reason `SystemKey` was chosen.

---

## §4 Set hierarchy (D3)

### 4.1 Representation

`set_parents: HashMap<SystemSetId, Vec<SystemSetId>>` (child → direct parents). A `Vec` of parents (not a single parent) because Bevy permits multi-set membership (research §1.2 `player_footsteps.in_set(A).in_set(B)`); by symmetry a set may nest in multiple parents. Direct members stay in the existing `set_members` (`schedule_builder.rs:69`).

### 4.2 Transitive membership flatten (the leaf-edge input for D1)

```rust
// Step 2.5 (NEW). Produces `transitive_members: HashMap<SystemSetId, Vec<SystemKey>>`
// where transitive_members[S] = every system in S OR in any set nested under S.
//
// Algorithm: per-set DFS over the REVERSED parent graph (parent → children),
// collecting direct members of the set and all descendant sets.
// Cycle detection: a SET HIERARCHY cycle (S in_set T, T in_set S) is caught
// here with ScheduleBuildError::SetHierarchyCycle BEFORE the system-edge Tarjan,
// matching Bevy's separate HierarchySort vs DependencySort (research §1.5 steps 1-2).
fn flatten_set_membership(
    direct_members: &HashMap<SystemSetId, Vec<SystemKey>>,
    set_parents:    &HashMap<SystemSetId, Vec<SystemSetId>>,
    n_sets: usize,
) -> Result<HashMap<SystemSetId, Vec<SystemKey>>, ScheduleBuildError> {
    // 1. Build child graph: for (child, parents) in set_parents, parent gets child.
    // 2. Detect cycle in the set-hierarchy DAG via iterative DFS color marking
    //    (WHITE/GRAY/BLACK). GRAY-revisit ⇒ SetHierarchyCycle{ sets: [..] }.
    // 3. For each set S (in ascending SystemSetId order — determinism, C2):
    //      collect direct_members[S] ++ union of transitive_members[child]
    //      for every child of S (memoized post-order so each set computed once).
    // 4. Sort each transitive_members[S] by SystemKey.0 and dedup
    //    (a system reachable via two nested paths appears once).
    Ok(transitive)
}
```

**Why a separate hierarchy cycle check** (not just "let Tarjan find it"): a hierarchy cycle never produces a *system* edge (membership alone has no edge — §2.1), so the system-level Tarjan would NOT see it. Bevy splits these for exactly this reason (research §1.9 `HierarchySort` ≠ `DependencySort`). The hierarchy DFS is O(V_sets + E_parents), trivial.

### 4.3 Nesting → leaf-edge expansion (worked)

`integrate.in_set(MovementSet)`, `configure_set::<MovementSet>().in_set::<PhysicsSet>()`, `configure_set::<PhysicsSet>().before::<RenderSet>()`, `render.in_set(RenderSet)`:
- `transitive_members[PhysicsSet] = {integrate}` (via MovementSet child).
- `transitive_members[RenderSet] = {render}`.
- `expand_set_edges` sees `SetBeforeSet(PhysicsSet, RenderSet)` → `{integrate → render}`.
- That single edge flows into Tarjan + Kahn → `integrate` precedes `render`. Correct transitive flattening with no set nodes in the DAG.

### 4.4 Edge cases

- **Empty set referenced in ordering**: `before_set::<S>()` where `members(S) = ∅` → `expand_set_edges` finds `transitive_members[S]` either absent (never interned → `EmptyOrUnknownSet`) or present-but-empty. Present-but-empty (set was `configure_set`'d but no member joined) → **0 edges + build warning** (§6.4), matching Bevy's "silently does nothing" but loud. Not an error: an empty well-known set (e.g. a user's `InputSet` with no input systems this build) is legitimate.
- **Self-nesting** `S in_set S` → 1-node hierarchy cycle → `SetHierarchyCycle`.
- **Diamond nesting** (S nests in A and B, both nest in C) → `transitive_members[C]` dedups S's members.

---

## §5 `#[derive(SystemSet)]` macro (D4) — extend existing to enums

### 5.1 Current state (already partly done)

The macro EXISTS at `boyko_macros/src/lib.rs:1150-1199`. It:
- Generates `impl ::boyko_ecs::ecs::core::schedule::SystemSet for #name {}` (`lib.rs:1194-1196`) — path verified against `mod.rs:33` re-export. ✓
- Accepts unit structs. ✓
- **Rejects enums** (`lib.rs:1173-1180`) and non-unit structs (`lib.rs:1185-1192`).

**Delta:** add enum support (each variant = distinct set), which forces the D6 keying change (variants share a `TypeId`, so identity needs a discriminant).

### 5.2 Generated code shape — unit struct (unchanged) + enum (new)

The `SystemSet` trait is methodless today (`system_set.rs:53`). To distinguish enum variants we add **one method** that yields the variant discriminant. Default impl returns 0 (covers all existing hand-written + unit-struct impls — no breakage):

```rust
// system_set.rs — trait gains ONE defaulted method (no breaking change)
pub trait SystemSet: Send + Sync + 'static {
    /// Distinguishes variants of an enum set. Unit-struct sets use the
    /// default (0); `#[derive(SystemSet)]` on an enum overrides per variant.
    /// Identity = (TypeId::of::<Self>(), set_discriminant(self)).
    #[inline]
    fn set_discriminant(&self) -> u32 { 0 }
}
```

Because `set_id_of_typed::<S>()` (§3.2) is called from `SystemConfig`/`ConfigureSet` where the user passes a **type** (turbofish), not a value, we cannot call `set_discriminant(&self)` there. Resolution: enum sets are targeted **by value**, unit sets by type. Two intern entry points:

```rust
// ScheduleBuilder (D6)
fn set_id_of_typed<S: SystemSet>(&mut self) -> SystemSetId;          // unit sets, disc=0
fn set_id_of_value<S: SystemSet>(&mut self, set: S) -> SystemSetId;  // enum variants, disc=set.set_discriminant()
```

…and the `SystemConfig`/`ConfigureSet` methods come in two flavors: `before_set::<S>()` (unit, type-level) and `before_set_value(s: S)` (enum variant, value-level). Given the cognitive cost, **decision (§12 OQ-1 resolved):** enum sets are the *less common* case; expose them via the value-taking methods (`in_set(MyEnum::Variant)` already takes a value at `system_config.rs:103`! — so `in_set` is value-based today). Therefore:

- `in_set` (existing, `system_config.rs:103`) — already takes `_set: S` by value. **Change its body** to call `set_id_of_value(set)` (consume `set`, read its discriminant) instead of `set_id_of(TypeId::of::<S>())`. This is the natural fit and requires no new method.
- `before_set`/`after_set` and `configure_set` — provide **value-taking** signatures (`before_set(s: S)`, `configure_set(s: S)`) so enum variants work uniformly. Unit structs are passed as `before_set(MySet)` (a unit value is free to construct). This drops the turbofish from §3.2/§3.3 sketches — finalize the signatures as value-based.

Revised signatures (final):
```rust
pub fn before_set<S: SystemSet>(self, set: S) -> Self;   // SystemConfig
pub fn after_set<S: SystemSet>(self, set: S) -> Self;
pub fn configure_set<S: SystemSet>(&mut self, set: S) -> ConfigureSet<'_>; // ScheduleBuilder
// ConfigureSet::before<T: SystemSet>(self, set: T) -> Self;  etc.
```

This also matches Bevy (`in_set(SomeEnum::Variant)`, `configure_sets(Update, SomeSet)` are value-based) and removes the unit-vs-enum entry-point split entirely. `set_id_of_value` is the single intern path.

### 5.3 Macro codegen for enums

```rust
// boyko_macros: extend system_set_macro to accept Data::Enum.
// For an enum, generate set_discriminant matching the variant index.
#[derive(SystemSet)]
enum CombatSet { Target, Damage, Cleanup }
// generates:
impl ::boyko_ecs::...::SystemSet for CombatSet {
    #[inline]
    fn set_discriminant(&self) -> u32 {
        match self {
            CombatSet::Target  => 0u32,
            CombatSet::Damage  => 1u32,
            CombatSet::Cleanup => 2u32,
        }
    }
}
```

Constraints (extend existing macro validation `lib.rs:1160-1192`):
- **Enum variants must be fieldless (unit variants only).** A variant with data has no stable type-level identity. Reject data-carrying variants with `compile_error!("SystemSet enum variants must be unit variants (no fields)")`.
- Keep the generics rejection (`lib.rs:1160-1167`) and union rejection.
- Unit-struct path unchanged (emits no `set_discriminant` override → trait default 0).
- `> u32::MAX` variants: impossible in practice; no guard (a `debug_assert` would never fire).

### 5.4 D6 keying change (enables enums)

```rust
// schedule_builder.rs — replace set_id_of(TypeId) with:
sets: HashMap<(TypeId, u32), SystemSetId>,   // was HashMap<TypeId, SystemSetId>

fn set_id_of_value<S: SystemSet>(&mut self, set: S) -> SystemSetId {
    let key = (TypeId::of::<S>(), set.set_discriminant());
    let next = self.sets.len();
    let id = *self.sets.entry(key).or_insert_with(|| SystemSetId(next));
    self.set_names.entry(id).or_insert_with(|| core::any::type_name::<S>());
    id
}
```

`set_id_of(TypeId)` (`schedule_builder.rs:117-123`) is removed; the single caller `system_config.rs:104` migrates to `set_id_of_value`. `set_id_of` on `SystemConfig` (`system_config.rs:120`) is removed (was an unused-by-prod convenience; `set_id_of_value` replaces it). The `derive_system_set_smoke.rs` test (`distinct_sets_have_distinct_typeids`) still passes: distinct unit structs → distinct `TypeId` → distinct `(TypeId, 0)` keys.

---

## §6 Diagnostics / missing-target (D5)

### 6.1 Error type + `try_build` / `build` split

Current `build` panics (`schedule_builder.rs:214`, message `boyko-B9001`). The existing style is **panic-with-precise-message**. Decision (§12 OQ resolved): introduce a `ScheduleBuildError` enum and a `try_build(self, world) -> Result<Schedule, ScheduleBuildError>`; keep `build(self, world) -> Schedule` as a thin wrapper that `.expect()`s with the formatted error. This preserves all existing call sites (`schedule.rs` tests, `phase9_scheduler.rs` bench call `build`) AND gives a non-panicking path for library users (CLAUDE.md notes `EcsMaster` returning `Result` is "questionable for a library" — `try_build` is the library-friendly door without forcing `Result` on everyone).

```rust
// schedule_builder.rs — NEW
#[derive(Debug)]
#[non_exhaustive]
pub enum ScheduleBuildError {
    /// before/after/chain cycle among systems (was boyko-B9001).
    OrderingCycle { systems: Vec<&'static str> },          // B9001
    /// Set-hierarchy cycle (S in_set T, T in_set S, ...).
    SetHierarchyCycle { sets: Vec<&'static str> },         // B9002
    /// before_set/after_set/configure_set referenced a set with no members.
    EmptyOrUnknownSet { set: &'static str },               // B9003 (also a warn path, §6.4)
    /// Two sets ordered relative to each other share a member (contradiction).
    SetsOrderedButIntersect { a: &'static str, b: &'static str, shared: &'static str }, // B9004
    /// before(key)/after(key) where key indexes outside this builder.
    UnknownSystemKey { key: SystemKey, n: usize },         // B9005
}
```

`build` formats: `panic!("boyko-{code}: {detail}")` — same prefix scheme as today (`boyko-B9001`).

### 6.2 Missing / foreign `SystemKey` (the silent-misindex fix)

Today `SystemConfig::before` (`system_config.rs:65`) pushes `OrderingEdge::Before(self.key, other)` with NO validation of `other`. If `other` is from a different builder or stale, `as_dag_edge` produces an edge with an out-of-range endpoint; `reorder[from_key.0]` (`schedule_builder.rs:262`) panics on OOB in debug, **silently mis-indexes in release** (research §6.3). Fix at build (validate ALL endpoints once, before Tarjan):

```rust
// Step 3 (modified): after collecting dag_edges_keys, before Tarjan:
for &(a, b) in &dag_edges_keys {
    if a.0 >= n { return Err(UnknownSystemKey { key: a, n }); }
    if b.0 >= n { return Err(UnknownSystemKey { key: b, n }); }
}
```

This is O(E) at build, negligible. It upgrades the release-mode silent corruption to a precise build error (a strict improvement over Bevy's silent no-op, research §6.3). A `SystemKey` from a *different* builder with a coincidentally in-range `.0` cannot be detected (it's just a `usize`); we accept this — the type is opaque and the borrow on `&mut ScheduleBuilder` in `SystemConfig` makes cross-builder mixing require deliberate effort. Documented as a known limitation.

### 6.3 Naming sets in cycle messages (§2.4 trade-off paid here)

When the system-level Tarjan reports an SCC that arose from set expansion, the message should name the involved sets, not just systems. Build a reverse map `system → Vec<SystemSetId>` from `set_members` (cheap), and for each system in the reported SCC, append its set memberships to the message. `SetsOrderedButIntersect` (§2.3) is detected *before* Tarjan with exact set+system names, so the common set-cycle case (two ordered sets sharing a member) already gets a precise message; only genuine multi-hop set cycles fall through to the enriched Tarjan message.

### 6.4 Empty-set warning

`expand_set_edges` encountering a present-but-empty `transitive_members[S]` emits a warning via `eprintln!` gated behind a `debug_assertions`-OR-feature flag (no logging dep in the hot crate; build is cold so `eprintln!` is acceptable here). Message: `boyko-W9003: set '{name}' is ordered but has no members; the ordering has no effect`. Matches Bevy's behavior (does nothing) but loud (research §6.3 recommendation). Not promoted to error — empty well-known sets are legitimate.

---

## §7 Sync-points decision (D7)

**CONFIRMED: `insert_sync_points` stays a no-op for Phase 15. Sound. Documented as a deferred parallelism optimization.**

Evidence (all in-code):
1. The pass-through is already documented correct under SCH7 (`schedule_builder.rs:333-347`): the apply-window barrier (`schedule.rs:316-384`) flushes EVERY system's `Commands` via `SystemParam::apply` inside the apply window, and a successor's `pred_remaining` is decremented only AFTER its predecessor's `apply` returns (`schedule.rs:364-372`).
2. Therefore deferred-command visibility across an ordering edge already holds for free (research §0 bullet 3, task hard-constraint 3): a successor ordered after a `Commands`-using predecessor always observes the flushed effects. **No `ApplyDeferred` node is needed and the task forbids adding one.**
3. Phase 15 only ADDS ordering edges (D1-D3). Each new edge gets a `pred_remaining` dependency + conflict bit through the **unchanged** `ConflictGraph::build` (`conflict_graph.rs:142-149`). The apply-window already serializes apply against the edge. So every Phase-15 edge inherits the same correct deferred-visibility semantics. No correctness gap is introduced.

**What we forgo (honest trade-off):** Bevy coalesces sync points by topological distance (research §1.8), running fewer, batched `ApplyDeferred` nodes — strictly more parallel than boyko's "one extra dispatcher round per deferred system" (`schedule_builder.rs:343-347`). boyko leaves this as a future optimization (the doc already files it as "Phase 9.1 follow-up", `schedule_builder.rs:349-353`). Phase 15 does not regress it and does not need it.

**Action:** update the `insert_sync_points` doc-comment to note Phase 15 re-confirmed the no-op is correct under the expanded edge set (no code change to the function body).

---

## §8 Cycle detection through sets + determinism (C1, C2)

### 8.1 Cycle-through-sets — fed into existing Tarjan (C1)

Two cycle classes, two detectors, both at build, ordered correctly:
1. **Set-hierarchy cycle** (`S in_set T, T in_set S`): caught in Step 2.5 `flatten_set_membership` (§4.2) → `SetHierarchyCycle`. Must run FIRST because a hierarchy cycle would make `transitive_members` ill-defined (infinite recursion) — the DFS color-marking stops it.
2. **Ordering cycle through sets** (`configure_set(S).before(T)` + `configure_set(T).before(S)`): after `expand_set_edges`, the combined `dag_edges_keys` (direct ++ expanded) contains the system-level edges `sᵢ→tⱼ` and `tⱼ→sᵢ`. The **existing** Tarjan (`schedule_builder.rs:209`) runs over this combined list and reports the SCC → `OrderingCycle` (enriched with set names, §6.3). The fast path for "two ordered sets share a member" is the pre-Tarjan `SetsOrderedButIntersect` check (§2.3).

Confirmation that expanded edges feed Tarjan: Step 3 builds `dag_edges_keys` = direct ++ expanded (§2.2); Step 4 `tarjan_scc(n, &dag_edges_keys)` (`schedule_builder.rs:209`) consumes exactly that vec. A `S before T, T before S` cycle yields system-level back-edges → SCC of size > 1 → existing panic path with a clear message.

### 8.2 The `n` (node count) is unchanged

Tarjan/Kahn operate over `n = descriptors.len()` (systems only) — `schedule_builder.rs:200`. Set expansion adds EDGES, never NODES (§2.4). So `tarjan_scc(n, ...)` and `kahn_topological_sort(n, ...)` need no signature change; they already accept an arbitrary edge list. **This is why no executor or graph-size change is required.**

### 8.3 0%-regression re-confirmation

- `run` path (`schedule.rs:116-194`), `executor_main_loop` (`:233`), `try_dispatch_ready` (`:425`), `reset_for_frame` (`executor_scratch.rs:161`): **none touched.** They read `pred_remaining`/`conflict_bits` produced at build.
- Phase 15 adds edges → `ConflictGraph::build` produces a (possibly) larger `pred_count[i]` and wider-populated `conflict_bits[i]`. But `conflict_bits[i]` is already a `FixedBitSet` of length `n` (`conflict_graph.rs:105`) — its SIZE does not grow with edges, only its set-bit density. `bitset_intersects` (`schedule.rs:458`) scans the same `n`-bit width regardless. So even a schedule WITH Phase-15 edges has identical per-round scan cost to one without. The "50 systems" bench (`phase9_scheduler.rs`) registers systems with NO set/ordering edges → its `dag_edges_keys` is empty → `expand_set_edges` returns `vec![]` → byte-identical build output to today → trivially 0% regression. Re-run the bench as a guard (§10).

### 8.4 Determinism (C2)

Kahn is FIFO-stable (`schedule_builder.rs:461-463`, `kahn_topological_sort` uses `VecDeque`). Determinism survives expansion iff the expanded edge list is itself deterministic:
- `set_ordering: Vec<SetOrderEdge>` — declaration order (push order). Deterministic.
- `transitive_members[S]` — sorted by `SystemKey.0` and deduped at the end of `flatten_set_membership` (§4.2 step 4). Deterministic.
- `expand_set_edges` iterates `set_ordering` in order, and within each set-pair iterates `members` in sorted order (§2.3). So `out` is deterministic.
- `HashMap` iteration is NOT used to produce edges (we iterate the `Vec<SetOrderEdge>` and sorted member vecs). `set_members`/`set_parents`/`sets` HashMaps are only used for *lookup*, never for *ordered iteration that affects edge order*. The one place we iterate a map for output — `flatten_set_membership` step 3 "for each set S" — is ordered **by ascending `SystemSetId`** (collect keys, sort), not by hash order.

Net: identical schedule across runs/machines/CPU-counts. This is the Phase-15 value proposition (research §6.2): pin intentional order so behavior is robust. boyko was already deterministic for unordered conflicting pairs (Kahn FIFO); Phase 15 keeps it.

---

## §9 Wave / Step plan

Steps are grouped into waves by dependency. Independent steps within a wave can be parallelized across developer agents (per memory: parallel developers for non-overlapping files).

### Wave 1 — Foundations (set identity + error type)
- **Step 1** — D6 keying: `ScheduleBuilder.sets: HashMap<(TypeId,u32), SystemSetId>`; replace `set_id_of` with `set_id_of_value<S>(set: S)`; add `set_names`; migrate `system_config.rs:104` `in_set` to `set_id_of_value`. *(schedule_builder.rs, system_config.rs)*
- **Step 2** — D5 error type: add `ScheduleBuildError` enum; add `try_build`; make `build` a wrapper that formats+panics with `boyko-B900x` codes. *(schedule_builder.rs)*
- **Step 3** — `SystemSet::set_discriminant` defaulted method on the trait. *(system_set.rs)*

### Wave 2 — Set-level API surface (depends on W1)
- **Step 4** — `OrderTarget` + `SetOrderEdge` in `ordering.rs`; `set_ordering`/`set_parents` builder fields. *(ordering.rs, schedule_builder.rs)*
- **Step 5** — `SystemConfig::before_set(set)`/`after_set(set)`. *(system_config.rs)*
- **Step 6** — `ConfigureSet` handle + `ScheduleBuilder::configure_set(set)` with `before`/`after`/`in_set`. *(schedule_builder.rs)*

### Wave 3 — Expansion engine (depends on W2) — the core deliverable
- **Step 7** — D3 `flatten_set_membership` (transitive members + hierarchy-cycle detection → `SetHierarchyCycle`). *(schedule_builder.rs)*
- **Step 8** — D1 `expand_set_edges` (system↔set + set↔set → pairwise edges; `SetsOrderedButIntersect` + empty-set checks). *(schedule_builder.rs)*
- **Step 9** — Wire Steps 7-8 into `build`: end the `_sets`/`_set_members` discard (`:159-160`); insert Step 2.5/2.6 between name-capture and edge-collection; combine direct ++ expanded into `dag_edges_keys`; add the `UnknownSystemKey` endpoint validation (§6.2). *(schedule_builder.rs)*

### Wave 4 — Diagnostics polish (depends on W3)
- **Step 10** — Enrich Tarjan cycle message with set names (§6.3); empty-set warning (§6.4); update `insert_sync_points` doc to record the Phase-15 no-op confirmation (§7). *(schedule_builder.rs)*

### Wave 5 — Macro (parallel with W2-W4; depends only on W1 Step 3)
- **Step 11** — Extend `#[derive(SystemSet)]` for enums: accept `Data::Enum` with unit variants; generate `set_discriminant` match; reject data-carrying variants. *(boyko_macros/src/lib.rs)*

### Wave 6 — Tests + validation (depends on all)
- **Step 12** — Unit + integration + trybuild + bench-regression (see §10). *(tests/, benches/)*

**Effort:** Roadmap estimates 1-2 weeks (`PHASE-13-ROADMAP.md:82`). With ~70% scaffolded, the net new code is ~6 functions + 1 enum + macro extension. Realistic: 5-6 working steps of implementation + test wave.

---

## §10 Test surface (for the tester)

### 10.1 Unit tests (in-module `#[cfg(test)]`)
- **InSet expansion correctness** — `a,b in SetS; SetS before SetT; x in SetT` → assert post-build order places a,b before x (extend the `topological_sort_respects_before` pattern, `schedule_builder.rs:626`).
- **System-relative-to-set** — `x.before_set(S)` with members {a,b} → x precedes both; `y.after_set(S)` → y follows both.
- **Set-before-set cartesian** — members(S)={a,b}, members(T)={c,d}, `configure_set(S).before(T)` → all 4 edges present (assert via `ConflictGraph.pred_count`/successor lists like `pred_count_matches_in_degree`, `conflict_graph.rs:291`).
- **Hierarchy flatten** — `a in M; M in P; configure_set(P).before(R); r in R` → a precedes r.
- **Diamond nesting dedup** — S in A and B, A,B in C → `transitive_members[C]` contains each member once.
- **set_id_of_value keying** — distinct enum variants → distinct `SystemSetId`; same variant → same id; unit struct → disc 0.

### 10.2 Cycle / error tests (`#[should_panic]` on `build`, `Err` on `try_build`)
- **Ordering cycle through sets** — `configure_set(S).before(T)` + `configure_set(T).before(S)` with shared-free members → `OrderingCycle` (B9001).
- **SetsOrderedButIntersect** — `S before T` where a system is in both → `SetsOrderedButIntersect` (B9004) with the precise pre-Tarjan message.
- **Set-hierarchy cycle** — `configure_set(S).in_set(T)` + `configure_set(T).in_set(S)` → `SetHierarchyCycle` (B9002).
- **Unknown SystemKey** — feed a `SystemKey(9999)` into `before` on a 2-system builder → `UnknownSystemKey` (B9005), in BOTH debug and release (the §6.2 fix).
- **Empty set warning** — `before_set(S)` with no members → builds successfully, no edge (assert `pred_count` all-zero); warning path covered by a smoke check.
- **Existing `cycle_in_before_after_panics`** (`schedule_builder.rs:602`) must still pass (B9001 via `try_build`-wrapper).

### 10.3 Macro (`trybuild`, tester-owned suite)
Add `tests/system_set_compile_fail/` (mirror `bundle_compile_fail.rs` harness, `tests/bundle_compile_fail.rs:23-28`):
- `generic_set.rs` → "does not support generics".
- `data_carrying_variant.rs` → "enum variants must be unit variants".
- `union.rs` → "can only be derived for...".
- `tuple_struct.rs` → existing "requires a unit struct" (regression guard).
- **Positive** enum smoke in `derive_system_set_smoke.rs`: `#[derive(SystemSet)] enum CombatSet { Target, Damage }` compiles; `Target.set_discriminant() != Damage.set_discriminant()`.

### 10.4 Bench regression (the 0%-regression gate)
- Re-run `benches/phase9_scheduler.rs` "50 systems" (no set/ordering edges). Expectation: byte-identical build output (`expand_set_edges` returns empty), `run` untouched → within noise of the 1.72×-vs-Bevy baseline. This is the §8.3 guard; a regression here means something leaked into `run` and the plan was violated.
- Optional: a NEW build-time micro-bench `schedule_build_with_sets` (1000 systems, 10 sets, cartesian ordering) to characterize expansion cost — informational, not a gate (build is cold).

### 10.5 Miri
- One Miri test (`tests/miri_phase15.rs`) building + running a schedule that uses set ordering. Phase 15 introduces NO new `unsafe` (all build-time, safe Rust over `Vec`/`HashMap`). Miri's role is to confirm the *executor* still behaves identically with the new edges — reuses the `miri_phase9.rs` single-thread discipline (multi-thread Miri deferred per Phase 9 note).

### 10.6 debug_assert! invariants (mandatory)
- `expand_set_edges`: `debug_assert!(combined_edges.len() <= MAX_EXPANDED_EDGES)` (§2.5).
- `flatten_set_membership`: `debug_assert!` each `transitive_members[S]` is sorted + deduped (determinism guard, §8.4).
- Post-expansion: `debug_assert!` every `dag_edges_keys` endpoint `< n` (release path is the §6.2 hard error; debug doubles as assert).
- Reuse existing `ConflictGraph::build` symmetry assert (`conflict_graph.rs:154-162`) — set edges flow through it unchanged.

---

## §11 Risk register

| ID | Risk | Likelihood | Impact | Mitigation |
|----|------|-----------|--------|-----------|
| R1 | Set expansion leaks cost into `run` | Low | Critical (breaks 0%-regression) | Architecturally impossible: expansion is in `build`; §8.3 bench gate catches any accidental `run` touch. Code review checklist: "did `schedule.rs`/`executor_scratch.rs` change?" must be NO. |
| R2 | `set_id_of` key change breaks existing `derive_system_set_smoke` / hidden callers | Med | High | Only caller is `system_config.rs:104` (grep-confirmed). `(TypeId,0)` for unit structs preserves distinctness. Run full `derive_system_set_smoke.rs` (3 tests) unchanged. |
| R3 | `try_build`/`build` split breaks existing `build` call sites (tests, bench) | Med | Med | `build` kept as a panicking wrapper with identical signature; all `schedule.rs` tests + `phase9_scheduler.rs` call `build` unchanged. Only the panic *message construction* moves behind the error enum (same `boyko-B9001` prefix → `cycle_in_before_after_panics`'s `#[should_panic(expected="boyko-B9001")]` still matches). |
| R4 | Enum-set value-vs-type API confusion (the §5.2 entry-point split) | Med | Med | Resolved to **uniform value-based** API (`before_set(set)`, `configure_set(set)`, `in_set(set)`) matching Bevy + existing `in_set` signature. No turbofish, single intern path `set_id_of_value`. |
| R5 | Cartesian blowup (set×set) OOMs build on a pathological schedule | Low | Med | `MAX_EXPANDED_EDGES` debug-assert (§2.5); `MAX_SYSTEMS_PER_SCHEDULE=1024` caps the absolute ceiling (~1M edges, ~50ms, cold). Realistic schedules ≤ ~900 edges/pair. |
| R6 | Set-cycle diagnostic names systems but not sets (poor UX) | Med | Low | §6.3 reverse `system→sets` map enriches the message; `SetsOrderedButIntersect` fast-path gives exact names for the common case. |
| R7 | Hierarchy cycle not caught (infinite recursion in flatten) | Low | High | `flatten_set_membership` uses iterative DFS color-marking (§4.2); GRAY-revisit → `SetHierarchyCycle` before any transitive computation. Unit test R-class covers it. |
| R8 | Foreign `SystemKey` (different builder, in-range `.0`) silently mis-targets | Low | Low | Cannot detect (opaque `usize`); documented limitation. `&mut ScheduleBuilder` borrow makes cross-builder mixing deliberate. Out-of-range case IS caught (§6.2). |
| R9 | Macro enum support regresses unit-struct path | Low | Med | Unit-struct branch unchanged (emits no `set_discriminant` override → trait default). `Data::Struct(Fields::Unit)` arm untouched; new `Data::Enum` arm added beside it. Existing smoke tests guard. |
| R10 | `set_discriminant` defaulted method silently breaks hand-written `impl SystemSet for X {}` | Low | Low | Method is **defaulted** (returns 0) — existing empty impls (`system_set.rs:81`, `derive_system_set_smoke.rs`) compile unchanged. Non-breaking trait extension. |

---

## §12 Open questions

1. **Value-based vs type-based set API — RESOLVED in-plan (§5.2).** Adopted uniform value-based (`before_set(set)`, `configure_set(set)`), matching the existing `in_set(set)` signature (`system_config.rs:103`) and Bevy. Flagging for critic sign-off: this drops the turbofish forms sketched in early §3; confirm no consumer wanted `before_set::<S>()` type-only ergonomics.

2. **`try_build` public surface.** Plan exposes both `build` (panicking, existing) and `try_build` (`Result`). Should `try_build` be the documented-preferred entry, or stay an advanced escape hatch? Leaning: document `build` for apps, `try_build` for libraries/tools that want to surface schedule errors gracefully (aligns with CLAUDE.md's "EcsMaster returning Result is questionable" note — give the choice, don't force it).

3. **`_ignore_deferred` ordering variants — OUT OF SCOPE (confirm).** Research §1.8 / open-Q4: boyko has no command-flush opt-out; every system flushes in its apply window (`schedule.rs:350`). A `before_ignore_deferred` would be a parallelism optimization, not correctness. Recommend deferring to the same future phase as coalesced sync-points (§7). Confirm exclusion.

4. **`configure_set` discoverability without an `EcsMaster` entrypoint.** Users build schedules via `ScheduleBuilder::new(pool)` (grep-confirmed; `EcsMaster` has no `schedule()` accessor). `configure_set` lives on `ScheduleBuilder` — fine. But should Phase 15 also add an `EcsMaster::schedule_builder()` convenience? Out of scope (no ECS-core change needed); flagging only because docs may want it.

5. **Keep the redundant conflict bit for pure ordering edges? — DEFER, benchmark-gated (research §4.2).** Phase 15 KEEPS the existing "ordering edge ⇒ conflict bit" rule (`conflict_graph.rs:146-149`) — it is tested, correct, and matches Bevy's "false conflict". Dropping it for pure (non-data-conflicting) ordering edges is a measurable micro-opt that should be its own benchmarked change, NOT bundled into Phase 15 (which must be 0%-regression). Confirm we do not touch this invariant now.

6. **Should `before_set` validate the target set is non-empty at *call* time?** No — members may join after the `before_set` call (the set is populated by later `in_set` calls; all known by `build`). Validation/warning happens at build (§6.4), matching Bevy's build-time flatten. Confirm the "populate-then-build" ordering assumption (research open-Q3: no incremental post-build system addition — `SCH1` forbids it, `executor_scratch.rs:109`).

---

### Files touched (all absolute paths)

**Modified:**
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\schedule\schedule_builder.rs` — D1/D3/D5/D6 (expansion, flatten, errors, keying, build wiring) — primary
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\schedule\system_config.rs` — `before_set`/`after_set`; migrate `in_set` to `set_id_of_value`
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\schedule\ordering.rs` — `OrderTarget`, `SetOrderEdge`
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\schedule\system_set.rs` — `SystemSet::set_discriminant` defaulted method
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\schedule\mod.rs` — re-export `ScheduleBuildError`, `ConfigureSet`, `OrderTarget`
- `D:\claude\BoykoEngine\crates\boyko_macros\src\lib.rs` — enum support in `system_set_macro` (`:1150-1199`)

**Unchanged (the hot path — explicitly NOT touched):**
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\schedule\schedule.rs` (executor)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\schedule\executor_scratch.rs`
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\schedule\conflict_graph.rs` (set edges flow through `build` unchanged)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\schedule\bitset_intersects.rs`

**New (tests):**
- `D:\claude\BoykoEngine\crates\boyko_ecs\tests\system_set_compile_fail.rs` + `tests\system_set_compile_fail\*.rs`
- `D:\claude\BoykoEngine\crates\boyko_ecs\tests\phase15_set_ordering.rs` (integration)
- `D:\claude\BoykoEngine\crates\boyko_ecs\tests\miri_phase15.rs`

---

**Checklist self-audit:** Plan structure (goal/metrics/justification/trade-offs) ✓. Data structures (`OrderTarget`/`SetOrderEdge` with repr, builder fields) ✓. API (value-based, no `dyn`, no internal leaks) ✓. Multithreading (N/A — all build-time, single-threaded dispatcher; hot path untouched, §8.3) ✓. Correctness (empty/MAX/cycle/foreign-key edge cases enumerated §4.4/§6/§11) ✓. Integration (files §end, `ConflictGraph`/`UnitId`-equiv compat verified) ✓. Validation (unit/trybuild/bench/miri/debug_assert §10) ✓. Two items marked RESOLVED in-plan (§12 Q1, Q5) with rationale for critic sign-off.

---

## §13 — Round 2 Patches (resolve critic Round 1; supersede conflicting earlier sketches)

> Critic Round 1 verdict: CHANGES REQUESTED — C1/C2/C3 (all one root cause: the
> §3.3/§3.4 *builder sketches* were never propagated to the §5.2 value-based
> resolution) + W1/W2 + O1/O2. The hot-path-untouched premise (§8) and the
> sync-points-stubbed decision (§7) were verified SOUND and are unchanged.
> Where these patches conflict with an earlier sketch, **the patch wins.**

### P1 (C1 + C3) — the ENTIRE set API is value-based; `set_id_of_typed` is deleted

The whole set surface takes the set/target/parent **by value** (so an enum
variant supplies its discriminant via `set_discriminant(&self)`). There is **one**
intern entry point, `set_id_of_value`; `set_id_of_typed` and the unused
`SystemConfig::set_id_of` (`system_config.rs:120`) are **deleted** — every §3.2/
§3.3/§3.4 turbofish sketch is superseded. Authoritative final signatures (this
block is the single source of truth):

```rust
// SystemConfig (per-system ordering targets)
impl SystemConfig {
    pub fn in_set<S: SystemSet>(self, set: S) -> Self;        // already value-based (system_config.rs:103)
    pub fn before_set<S: SystemSet>(self, set: S) -> Self;    // NEW
    pub fn after_set<S: SystemSet>(self, set: S) -> Self;     // NEW
    // existing before/after(SystemKey) + chain() unchanged
}

// ScheduleBuilder — set-level config
impl ScheduleBuilder {
    pub fn configure_set<S: SystemSet>(&mut self, set: S) -> ConfigureSet<'_>;
}

// ConfigureSet — set-vs-set ordering + set hierarchy. ALL targets/parents BY VALUE.
impl<'a> ConfigureSet<'a> {
    pub fn before<T: SystemSet>(self, set: T) -> Self;   // value, not turbofish
    pub fn after<T: SystemSet>(self, set: T) -> Self;    // value
    pub fn in_set<P: SystemSet>(self, parent: P) -> Self; // value — enables enum-variant set hierarchy
}

// The SOLE intern path. Reads the discriminant from the value.
impl ScheduleBuilder {
    fn set_id_of_value<S: SystemSet>(&mut self, set: S) -> SystemSetId;
}
```

`SystemSetId = (TypeId::of::<S>(), u32)` where the `u32` is `set.set_discriminant()`
(defaulted `0` for unit structs; the derive emits the variant index for enums).
**`SystemSet` does NOT gain `Clone`/`Copy`** — fieldless construction at each call
site (`E::Combat`) is the intended zero-cost pattern; a value is interned once per
call and only its `(TypeId, discriminant)` is stored. `ConfigureSet` stores only
the resolved `SystemSetId` (so the chained `.before(set)` consumes its own fresh
value, no re-use of a moved value). **Invariant:** the config path
(`configure_set(E::A)`) and the membership path (`in_set(E::A)`) both route through
`set_id_of_value`, so they resolve to the **same** `SystemSetId` — this is the C1
fix (an enum-variant set ordered via `configure_set` now orders the members that
joined it).

### P2 (C2) — unified empty-set policy; expansion consumes the transitive (D3) output

A referenced set is **always interned on first reference** (any of `in_set` /
`before_set` / `configure_set` calls `set_id_of_value`), so it always has a
`SystemSetId` — but it may have **zero members**. The `EmptyOrUnknownSet` *error*
is **removed from the ordering-expansion path**. The single uniform rule:

```rust
// In expand_set_edges — members(S) defaults to empty, never errors:
let members = |s: SystemSetId| transitive_members.get(&s).map(Vec::as_slice).unwrap_or(&[]);
// empty set on either side of a set-ordering edge => 0 expanded edges + ONE build WARNING
// (boyko-W15xx "set ordered but has no members"), matching §4.4/§6.4 — never an error, never non-deterministic.
```

`expand_set_edges` **consumes the D3 `transitive_members` output** (NOT the direct
`set_members`), so nested membership is fully flattened before expansion. The
`SetsOrderedButIntersect` check (a system transitively in both sides of `S before T`)
runs on these **post-flatten** lists. Contract asserted at the call site (cross-ref
§10.6): each `transitive_members[S]` is **sorted ascending by `SystemKey.0` and
deduped** (`debug_assert!`). D3 (flatten) is ordered strictly before D1 (expand) in
`build`.

### P3 (W1) — determinism invariant pinned to `SystemKey.0`

Expanded edges are ordered by a key **monotonic in pre-build insertion order
(`SystemKey.0`)**, because Kahn's FIFO tie-break (`schedule_builder.rs:461-463`) is
defined over insertion order and the expanded `(SystemKey, SystemKey)` edges enter
the **same** `dag_edges_keys` vec consumed by Kahn *before* the `reorder`
permutation. The §10.6 `debug_assert!` asserts "each `transitive_members[S]` sorted
by `SystemKey.0` ascending" (not merely "sorted"). A `HashSet`-derived dedup order
is **forbidden** (it would break byte-identical-across-runs determinism).

### P4 (W2) — honest bound: `MAX_SYSTEMS_PER_SCHEDULE` is the release rail

`MAX_EXPANDED_EDGES` is a **debug-only** early-warning (`debug_assert!` vanishes in
release — CLAUDE.md). The **real** release bound is `MAX_SYSTEMS_PER_SCHEDULE = 1024`
(N is capped; `pred_count: u16` `checked_add` panics on overflow,
`conflict_graph.rs:142-144`), which bounds the worst-case expanded edge count. §2.5
no longer calls `MAX_EXPANDED_EDGES` a "rail." Build-time cost budget corrected:
each expanded ordering edge ALSO sets a conflict bit (`conflict_graph.rs:146-149`),
so `ConflictGraph::build`'s existing O(N²/w) access scan (~1M iters at N=1024) is
joined by O(E) edge-bit insertions — at the worst case this ~doubles the dominant
build cost. Still **cold + one-shot + bounded** (build, never per-frame); the §8
0%-regression-on-`run` guarantee is unaffected.

### P5 (O1, optional but adopted) — enum-variant names in diagnostics

`SystemSet` gains a defaulted `fn set_name(&self) -> &'static str { type_name::<Self>() }`;
the enum derive overrides it to return `"Type::Variant"` per variant. `set_id_of_value`
stores the name alongside the id so `SetHierarchyCycle` / `SetsOrderedButIntersect`
messages distinguish variants of the same enum (otherwise all variants print as the
bare type name). Identity/correctness is unaffected either way; this is message
quality for the new enum-set feature.

### P6 (O2) — added tests
- **config-vs-membership agreement:** members joined via `in_set(E::A)` are ordered
  by `configure_set(E::A).before(E::B)` (proves both paths resolve to one `SystemSetId`).
- **enum-variant set hierarchy:** `configure_set(E::Child).in_set(E::Parent)` flattens
  so `E::Child`'s members are transitively in `E::Parent` and obey `E::Parent`'s ordering.

### Round 2 changelog
| Critic item | Severity | Disposition | Patch |
|---|---|---|---|
| C1 enum config-vs-membership key mismatch | CRITICAL | whole set API value-based; `set_id_of_typed` deleted | P1 |
| C2 empty-set policy inconsistent + intersect on direct lists | CRITICAL | uniform empty→0-edges+warning; expand consumes D3 transitive | P2 |
| C3 `ConfigureSet` methods take no value (can't express enum parents/targets) | CRITICAL | all `ConfigureSet` methods by value | P1 |
| W1 determinism invariant loose | HIGH | pinned to `SystemKey.0`; assert tightened | P3 |
| W2 `MAX_EXPANDED_EDGES` debug-only mislabeled "rail" + omitted cost | HIGH | real rail = `MAX_SYSTEMS_PER_SCHEDULE`; cost corrected | P4 |
| O1 enum variants indistinct in diagnostics | LOW | `set_name` defaulted + derive override | P5 |
| O2 missing cross-path tests | LOW | added | P6 |

Verified-sound and unchanged (critic "Positive"): hot path untouched (§8), sync-points
no-op (§7), no separate set-node graph (§2.4), `UnknownSystemKey` build-time validation
(§6.2), `build`/`try_build` split, defaulted `set_discriminant`.

### §13.1 — Round 3 corrections (resolve critic Round 2; supersede on conflict)

> Critic Round 2 confirmed C1/C2/C3/W1/W2/O1/O2 RESOLVED in mechanism, found
> C-NEW-1 (CRITICAL spec contradiction) + W-NEW-1 (HIGH) + OQ-1 (cleanup).

**R3-A (C-NEW-1) — `SystemSetId` is the EXISTING sequential `usize` newtype; P1's "SystemSetId = (TypeId, u32)" wording is corrected.**
`SystemSetId` stays exactly as today: `#[repr(transparent)] pub struct SystemSetId(pub usize)` (`system_set.rs:31`) — a dense sequential index. It is **interned** from the key `(TypeId::of::<S>(), set.set_discriminant())` via the `sets: HashMap<(TypeId, u32), SystemSetId>` map (§5.4 form), where `set_id_of_value` does `*self.sets.entry(key).or_insert_with(|| { let n = self.sets.len(); SystemSetId(n) })`. §13-P1's literal "`SystemSetId = (TypeId, u32)`" was imprecise shorthand for "interned from the `(TypeId, discriminant)` key" — the §5.4 representation WINS. C1 still holds: both `configure_set(E::A)` and `in_set(E::A)` compute the identical `(TypeId, discriminant)` key → the same interned `SystemSetId`. This preserves the existing public type, `repr(transparent)`, the 8-byte map-key cost, and the `system_set_id_equality_and_hash` test. `OrderTarget::Set(SystemSetId)` and `SetOrderEdge`'s fields use this `SystemSetId(usize)` unchanged.

**R3-B (W-NEW-1) — the empty-set warning is driven off the ORDERING-EDGE iteration, not off a `transitive_members` entry.**
In `expand_set_edges`, when processing a `SetOrderEdge` (or a system-before-set / set-before-set), look up `members(set) = transitive_members.get(&set).map(Vec::as_slice).unwrap_or(&[])`; **if the slice is empty, emit `boyko-W15xx` "ordering references set {name} which has no members (no system joined it via in_set)"** — regardless of whether `set` has a `transitive_members` key. This catches the never-`in_set`'d target (the silent-no-op footgun the critic flagged). Additionally, `flatten_set_membership` (D3) SEEDS `transitive_members` with an empty `Vec` for **every interned set id** (iterate the `sets` map), so the present-but-empty branch is uniform and the warning always has a name to print. Both together guarantee: any set named in an ordering edge but never joined produces a loud build warning, never a silent 0-edge no-op.

**R3-C (OQ-1) — `EmptyOrUnknownSet` is DELETED.**
Remove the `ScheduleBuildError::EmptyOrUnknownSet` variant (former B9003) from §6.1 and the `ok_or(EmptyOrUnknownSet(s))?` in the §2.3 pseudo-code. The ordering-expansion path never errors on an empty/absent set — it defaults `members()` to `&[]` and warns (R3-B). The only set-related build *error* remaining is a cycle (Tarjan, B9001) and `UnknownSystemKey` (§6.2, foreign-builder key). An interned-but-memberless set is a warning, never an error.

**Status after §13.1: APPROVED for implementation** (critic Round 2's stated bar — "once C-NEW-1, W-NEW-1 resolved and OQ-1 cleaned up" — is met; the executor/hot-path, sync-points, and set-node-graph decisions remain verified sound).
