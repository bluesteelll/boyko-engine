# Architecture: kernel lane K (deterministic apply, access completeness, ambiguity detection)

Tree: `D:/wt/joltab` @ `47c5dabd`. Every `file:line` below is from that tree. Paths are under `crates/boyko_ecs/src/ecs/core/` unless another crate is named.

## Goal

- **K1.** Every side effect that becomes visible at the apply window is applied in an order fixed by the build: ascending post-topological index within a wave, with the wave order already fixed. The order must not depend on completion order or on the worker count W. Cost: O(k + ⌈n/64⌉) per window, no allocation.
- **K2.** The two ways the enable column is touched become visible to analysis:
  - `Enabled<T>` / `Disabled<T>` read the column.
  - Deferred enable toggles write it.
  
  Without this, the pair behind the F1 diagnosis (`visibility_sync` vs `gather_mesh_draws`) was flagged only because the gather happens to be exclusive. K2 makes it a named conflict on `RenderEnabled`.
- **K3.** At build time, the schedule computes the pairs of systems whose access conflicts but which have no ordering path between them. There is an API to read them, a level setting that is off by default and costs nothing when off, allow-lists that require a written reason, and a ratchet test over the shipped host so the count can only fall.

## Context and constraints

- **The unified plan already names two of these pieces.** K1a is KC-36/D-E0 (`docs/unification/UNIFIED-SYSTEM-PLAN-01-KERNEL-CONTRACT.md:188`, `02-ORDER-OF-WORK.md:161`). K1b is D-E20 (`02:181`). This lane builds both as designed, with a stricter gate. The one deviation is in D2 (writers outside any schedule use the dispatcher lane). Landing them here satisfies those two plan rows ahead of their stated prerequisite, B1–B3 (see Open questions).
- **Invariants that must still hold:**
  - SCH6: `pending` is 0 at the end of each frame.
  - SCH7: the apply-window gate `pending > 0 && (pending == running || running == 0)` at `schedule/schedule.rs:778`.
  - EM2′-K.
  - ADG1 at `schedule.rs:1691-1698`.
  - The 256 B size pin on `SystemMeta` (`system/system_meta.rs:63-66`).
  - The 192 B size pin on `Access` (`system/access.rs:61`).
  - No field offset in the `Schedule` hot prefix may move (`schedule.rs:193-199`).
- **Owner rules that apply:**
  - Replays must play identically at any W and on any machine, and entity ids are outside that contract (U-20, Q-9).
  - No allocation on the hot path.
  - Principle 0.
  - Every gate must be able to fail.

**Measured today** (from `order_diag.md`):
- The wave split and dispatch order are identical across frames, runs and W ∈ {1, 3, 16}.
- The apply order inside a wave is not: 34 distinct orders in 34 frames at W = 3 and W = 16, and 1 at W = 1.
- The hwrt `taa_armed` host has 236 ambiguous pairs: 203 involve an exclusive system (EXCL) and 33 are DATA conflicts. That host includes two fixture systems (`drive_camera_motion`, `drive_moving_caster_motion`) that are not part of `EnginePlugins`.

## K1: deterministic application

### D1. The apply window drains first, then applies in ascending index (K1a = KC-36)

**What.** `apply_window_drain` (`schedule.rs:881-940`) is split into two phases.
- **Phase 1** pops all `target` completions into a preallocated window bitset. No user code runs in this phase, so ADG1 still holds exactly: `drained == target` when it ends.
- **Phase 2** walks the bitset in ascending word order with `tzcnt`. For each system it runs today's per-system body unchanged: clear `running`, `apply`, `drain_deferred_hook_queue`, mark `completed`, decrement successors.
- `ApplyDrainGuard` stays alive across phase 2. Its `Drop` still performs the single `fetch_sub(target)`.
- The bitset is the dead field `ExecutorScratch::ready_scratch` (`schedule/executor_scratch.rs:424-429`), renamed to `apply_window`. It is only ever cleared (`:612`) and read by nothing, so `ExecutorScratch` does not change size and nothing new is allocated.

**Why this is deterministic.** The window fires only when every running system has completed. `try_dispatch_ready` scans `0..n` with no W term (`schedule.rs:1288-1341`). So the set of systems in a window is fixed by the build and the run-condition results; only the pop order depended on timing. Sorting the set removes that dependence.

The following all happen inside `apply`, so they now follow ascending order:
- the order commands run in;
- table-row order of spawns;
- the order `ArchetypeId`s are created;
- dense and group slots;
- the order the free stack is pushed;
- hooks and observers across systems;
- enable-bit writes;
- sends from the apply path to the dispatcher lane.

**Rejected alternatives:**
- Sorting the popped indices in a `Vec`: O(k log k) and a second buffer, where the bitset is free.
- Iterating `running.ones()` and throwing the pops away: this depends on the `pending == running` arm of the gate, and the `running == 0` arm would apply nothing.
- Recording dispatch order in `try_dispatch_ready`: it gives the same order but ties the drain to the internals of dispatch.
- Bevy's `unapplied_systems` with separate sync points: our barrier is already per wave.

**Trade-off.** None measurable. See the cost table.

### D2. Event lanes are keyed by writer (K1b = D-E20)

**What.**
- `EventWriterState.thread_count` becomes `lane`, assigned at `init_state` in build order. The state stays 24 B (`system/params/event_writer.rs:50-63`).
- `send` and `send_many` stop reading the worker id (`:131-133`, `:162-164`).
- The swap still concatenates lanes in lane-index order (`events/event_dispatcher.rs:630-637`). The dispatcher lane stays last.
- `EcsMaster::events().send_event` becomes dispatcher-only (U-21). Called on a worker it returns `Err(EventSendOffDispatcher)` (`event_dispatcher.rs:285-292`).
- **Addition to D-E20:** an `EventWriter` state initialized outside a `ScheduleBuilder` (`run_system`, observers) uses the dispatcher lane instead of minting a new one. Those callers hold `&mut EcsMaster`, so the lane still has one writer. Without this, one-shot systems would grow the lane count at runtime.

**Why build order is enough.** Lane order is a function of registration order, which is fixed by the binary.
- Per-lane capacity now applies per writer, so which events are refused no longer depends on W (H-02).
- Only one thread writes a lane at a time: `EventWriter` is `&'s mut`, and the scheduler never runs one system instance on two threads.

**Rejected alternatives:**
- Keep worker lanes and sort at swap by (writer, sequence): +4–8 B per event plus a sort per swap. This is MQ-21's overturn form.
- Lanes keyed by post-topological index: needs one index space across Main and Fixed and a remap. Build order is just as much a function of the build.
- Sending everything through commands: +18 ns per event, and all sends get serialised into the apply window.

### D3. Entity ids are not made deterministic

**What.**
- Ids are claimed on the worker when a command is enqueued (`system/params/commands.rs:169-173` → the entity counter). That claim stays dependent on timing.
- The id **set** is deterministic, because the number of claims per window is fixed and claims drain the recycled stack before minting fresh ids.
- The id → (system, ordinal) mapping is not deterministic.
- The K1a gate asserts the id set and the (system, ordinal) → row mapping. It does not assert which id each spawn got.

**Why.** Owner ruling Q-9 (2026-09-17): replays key on stable replay keys, not on `Entity`.

**Rejected alternatives:**
- Claiming ids at apply time: breaks the synchronous `spawn().id()` contract.
- Per-system id leases: the priced revival MQ-19. Not requested.

### D4. Enable-bit writes need no mechanism of their own

`set_enable_bit` takes `&mut self` (`ecs_master/enable_tag_api.rs:149`). Every enable write therefore comes from one of three places:
- an apply window: `EnableTagCommand`, custom commands, hooks;
- an exclusive system run inline;
- setup code.

No param writes the enable column from a worker. D1 therefore fixes their order.

## K2: access completeness

### D5. Enable filters declare a read of the tag's column

**What.** `Enabled::init_access` and `Disabled::init_access` (`iters/query/filter_enable.rs:181-205`, `:357-362`) call `add_component_read(state.id)`, which is what `With<T>` does (`iters/query/filter.rs:569-573`). The ENBL-ACCESS-1 comment is rewritten: the filter really does read the column during the parallel phase, which is why the bit is declared.

**Why this changes no dispatch.**
- A conflict needs a write bit on the same id. No non-universal system can hold a component write on a bitset id: bitset ids have no pool, so no data param resolves against them.
- The only new conflict bits are therefore between an exclusive or `GpuCompute` system and a system whose access was previously empty.
- Those bits are never consulted:
  - EXC2 requires `running == 0` before a dispatcher-solo system is accepted (`schedule.rs:1326`).
  - The inline exclusive path sets and clears its `running` bit without spawning anything in between (`:1370-1451`).
- So the dispatch sequence is byte-identical and no golden moves.
- **Premise check:** a new `debug_assert!` in `FilteredAccessSet::finalize` (`system/filtered_access_set.rs:317-319`): for a non-universal system, `component_writes ∩ bitset_ids == ∅`.

**Forward compatibility.** When D7's worker-side marking lands, this read bit is exactly what its writes must conflict with.

**Rejected alternative.** A separate cold "order read" mask would be duplicate plumbing for a bit that is simply true.

### D6. Dispatcher-solo exclusives keep the access they actually declare

**What.**
- `FilteredAccessSet` gets a `declared: Access`. Every `add_*` ORs into it, even after `mark_universal` has made later adds no-ops (`:301-304`, FIX-1/X2).
- `NonSendRes` and `NonSendResMut::init_access` (`system/params/nonsend_res.rs:98-110`, `nonsend_resmut.rs:75`) record their `NonSendResourceId` as a read or a write.
- At `finalize`, if the set is universal **and** `meta.requires_dispatcher`, the declared access and the NonSend reads and writes go into `OrderMeta` (D8).

**Why.** Five of the seven exclusive systems in Main are exclusive only because they take a `NonSend` param: `apply_refcount_deltas`, `validate_asset_refs`, `gather_shadow_casters`, `reduce_caster_bounds`, `gather_mesh_draws` (`boyko_render/src/asset_refcount.rs:89,401`, `csm_caster.rs:210,472`, `mesh_draw.rs:1238`). Their effect on the world is exactly their other params. See D12.

### D7. Deferred writes are declared by construction and never by configuration

**What.** A deferring system is either **closed**, with a known set of written ids, or **open**, with unknown effects.

- **Typed param `EnableCommands<'s, T>`** (new file `system/params/enable_commands.rs`):
  - It owns a `CommandQueue` and has `HAS_DEFERRED = true`.
  - Its `init_access` calls `meta.declare_deferred_write(T::component_id())`.
  - Methods: `set(Entity, bool)` and `set_by_id(EntityId, bool)`. The second resolves the entity at apply time and does nothing if it is dead. This is `boyko_scene`'s `SetRenderEnabledById` contract (`boyko_scene/src/visibility_sync.rs:84-106`), moved into the kernel as `EnableTagByIdCommand`. ⚠ **Superseded 2026-09-21 (rungs A9 / A9b)**: `SetRenderEnabledById` no longer exists — it is `SetRenderEnabled`, keyed by the full `Entity` resolved at enqueue through the `Entities` param. The by-id form re-resolved the row at apply and was hazard H-06: a toggle pending for a despawned `E` landed on the `F` spawned on `E`'s recycled id in the same frame (`boyko_scene/tests/visibility_sync_gates.rs` gates 5–6). `boyko_render`'s two copies of the pattern (`asset_refcount.rs` `DisableStaleMeshCommand`, `snap_interpolation.rs` `DisableSnap`) were fixed the same way (`boyko_render/tests/deferred_toggles_recycled_id.rs`). A kernel `set_by_id` that resolves at apply would move H-06 into the kernel; a by-id surface, if it is still wanted, resolves the generation at ENQUEUE.
  - The declaration is exact because the param cannot touch any other tag.
- **Untyped `Commands`:** `init_access` (`commands.rs:405-415`) calls `meta.mark_deferred_untyped()`, which makes the system open.
- **Hand-written `System` impls** with `has_deferred() == true` are open unless their `initialize` calls the public `SystemMeta::declare_deferred_write`. That makes them closed. This is trusted, in the same way `has_deferred` already is (`system/system.rs:190`).
- **Effective status:**
  - `!may_defer` → no deferred writes.
  - `untyped || nothing declared` → open.
  - Otherwise → closed with the declared mask.

**Research.**
- Bevy's ambiguity checker cannot see `Commands` at all. That blindness is how F1 happened.
- flecs declares command writes with `[out] Transform()` / `.write<Transform>()`, and its pipeline inserts sync points from those declarations. The annotation is trusted.
- We take flecs' shape for typed params but not its trust: a config-level `.defers_write::<T>()` that closes a `Commands` holder would let a declaration lie and silently drop a pair. It is rejected.
- A debug-build audit at apply time was also rejected. It needs a new `thread_local!`, which counts against the D-M6 census, and it only checks paths that a test happens to exercise. Once closing is only possible by construction, it is not needed.

### D8. Rule for undeclared writers; where the metadata lives

**Open rule.** An open deferrer is assumed to write every enable tag. It is paired with every reader, and every deferred writer, of any enable tag.

**Why only enable tags.** They are the one component class whose only non-exclusive writer is an apply window, so access analysis has no coverage of them at all. Structural effects of untyped `Commands` (spawn, insert, remove of archetypal components) stay a named limit. Covering them would pair every `Commands` holder with every reader. The declaration mask already accepts any `ComponentId`, so widening the rule later needs no new plumbing.

**Metadata.** `SystemMeta` gets `order: Option<Box<OrderMeta>>`:
- It goes into the tail padding: `zone` ends at 243, so the field sits at offset 248, and 248 + 8 = 256. The size pin holds.
- It is `None` for plain systems (the 0% gate).
- It is allocated at init for `Commands` holders, typed deferred params and dispatcher-solo systems. That is about 10 on the host, at about 330 B each.

## K3: ambiguity detection

### D9. The analysis runs at build, on the post-topological DAG

- **Placement:** after `ConflictGraph::build` (`schedule/schedule_builder.rs:654`), which is the point where `conflict_graph.successors` exist, and before step 10 consumes the descriptors (`:664-687`). It lives in a `#[cold] #[inline(never)]` function in the new file `schedule/ambiguity.rs`.
- **Reachability:** a reverse topological sweep. `reach[i] = ∪ over successors s of ({s} ∪ reach[s])`. Indices are topological, so `s > i` and `reach[s]` is already complete. Set edges are already expanded into `dag_edges_keys` (`:515-525`), so paths through sets are covered.
- **Pairs:** for every `i < j` with `!reach[i][j]`, apply `allow`, then the pair predicate (D10b).

### D10. The pair predicate (added in K2a as `schedule/order_conflict.rs`, used by K3a)

**Each system's view:**
- world-exclusive (universal access without `requires_dispatcher`): unknown;
- dispatcher-solo: its declared access plus NonSend reads and writes (D6);
- anything else: its `Access`;
- the reader side also includes the reads of the system's own run conditions and of the conditions of any set that gates it.

**Classes, strongest first; all items are reported:**

| Class | Condition | Items |
|---|---|---|
| `WorldExclusive` | either side is world-exclusive | none |
| `Data` | declared read/write or write/write intersection over components and resources, plus NonSend ids between two dispatcher-solo systems | named ids with R/W on each side |
| `Deferred` | a closed deferred mask intersects the other side's reads, writes or deferred mask | ids |
| `DeferredOpen` | one side is open and the other side reads, or defer-writes, any bitset id (open × open counts) | those bitset ids |

**Events are excluded.** Readers read the buffer flattened at the swap (`event_dispatcher.rs:447`). Within one run of a schedule, the order of reader and writer changes nothing a reader sees. `every_tick` (D-E8) swaps between substeps, not within one.

### D11. Level setting and API

- `AmbiguityDetection { Off (default), Collect, Warn, Error }`, set per `ScheduleBuilder` and per `App`.
- `Off` computes nothing and stores `None`.
- `Warn` emits one new `boyko_log` warning code per pair.
- `Error` makes `try_build` return `ScheduleBuildError::Ambiguities`, and `build` panics with a new B code.
- `EnginePlugins` maps `BOYKO_SCHEDULE_AMBIGUITY=off|collect|warn|error`. An unrecognised value reports through the existing `W3009` path.
- **The default is `Off` in every profile.** Hundreds of test apps build schedules. `Warn` by default would log pairs that fixtures create on purpose, and would cost O(n²) per build. The ratchet (D13) is the enforcement; the environment variable is the interactive tool.
- Bevy also defaults to `Ignore` and enforces zero ambiguities in CI.

### D12. Exclusive pairs: world-exclusive ones are counted, dispatcher-solo ones are analysed

- **World-exclusive systems** (`&mut EcsMaster`, e.g. `propagate_transforms` at `boyko_scene/src/propagation.rs:222`) can touch anything. Every pair with no ordering path is reported. This is Bevy parity.
- **Dispatcher-solo systems** are exclusive for a threading reason (the `!Send` slab), not a data reason. Their pairs are reported only when their declared access, NonSend ids or deferred masks conflict.
- **Why not count them all:** it would add roughly 5 × 32 pairs whose order has no effect on data. A report that noisy trains authors to allow-list blindly.
- **Expected on the shipped composition:**
  - world-exclusive pairs ≈ 59: `propagate_transforms` 30 + the `LightingPlugin` seed closure 30 − 1 counted twice. This assumes the seed closure takes `&mut EcsMaster`. If it is NonSend-driven instead, the figure is 30.
  - The old EXCL remainder shrinks to its `Data` and `Deferred` subset. K3b measures it.
- **Rejected:** giving world-exclusive systems a declared scope. It would be unverifiable text; see the follow-ups.

### D13. Allow-lists require a reason and never name "all"

- **Methods:**
  - `SystemConfig::ambiguous_with(SystemKey, reason)`
  - `SystemConfig::ambiguous_with_set(set, reason)`
  - `ConfigureSet::ambiguous_with_set(set, reason)`: covers members × members. When the target is the configured set itself, it covers the pairs within the set.
  - `ScheduleBuilder::allow_ambiguous_component::<C>(reason)`
  - `ScheduleBuilder::allow_ambiguous_resource::<R>(reason)`
- **Item-level semantics:** a pair whose items are all allowed is dropped. `WorldExclusive` pairs have no items, so only system-level or set-level allows drop them.
- An empty reason is a build error (new B code).
- **No `ambiguous_with_all`:** it hides pairs that do not exist yet, which defeats the ratchet.
- Allow-lists affect diagnostics only; execution is untouched.

### D14. Ratchet gate

- Canonical pin line: `min(name_a, name_b)|max(name_a, name_b)|Class`, sorted, compared as a **multiset**, because closure systems can share a `type_name`.
- Topological indices are deliberately not pinned: they shift on unrelated edits.
- Item lists are printed when the test fails, not pinned, so an unrelated access edit does not churn the file.
- A second section pins allowed pairs with their reason text.
- **Comparison is exact in both directions.** A new pair is red. A pinned pair that is gone is also red, with an instruction to delete the line, so a fixed pair cannot quietly come back later.
- A `BASELINE_PAIRS` constant in the test must be ≥ the pin count and may only go down.
- The pin file may grow only in a commit that changes the **detector**, and that commit must say so.

## Data structures

```rust
// executor_scratch.rs:424-429 — renamed, same type and size.
pub(crate) apply_window: FixedBitSet,   // n bits; dispatcher-owned; phase 1 fills it, phase 2 zeroes it word by word

// system_meta.rs — one field appended after `zone`; offset 248; size_of stays 256 (pin :63-66).
pub(crate) order: Option<Box<OrderMeta>>,  // None = 0% gate

#[repr(C)]
pub(crate) struct OrderMeta {              // heap, written once at init, read once at build
    declared: Access,                      // 192 B: dispatcher-solo only (D6), valid iff flags & DECLARED
    deferred_writes: ComponentMask,        // 64 B: typed deferred params / hand-written declarations (D7)
    nonsend: Box<[(u32, bool)]>,           // (NonSendResourceId, is_write), usually 1 entry
    flags: u8,                             // DECLARED | DEFERRED_UNTYPED
}

// filtered_access_set.rs:122-135 — transient at init: + declared: Access (192 B)

// schedule.rs — appended after `world_id` (M3); the hot prefix does not move
pub(crate) analysis: Option<Box<ScheduleAnalysis>>,
pub(crate) struct ScheduleAnalysis { pairs: Box<[AmbiguousPair]>, allowed: Box<[AllowedPair]> }

// system_descriptor.rs:39-71 (build-time only)
pub(crate) ambiguous_with: Vec<(AllowTarget /* Key | Set */, &'static str)>,
// ScheduleBuilder (:101-145): ambiguity: AmbiguityDetection (1 B),
//   set_ambiguity: Vec<(SystemSetId, SystemSetId, &'static str)>,
//   allowed_items: Vec<(ItemId, &'static str)>
```

## Public API

```rust
// K2
pub struct EnableCommands<'s, T: Component> { /* &'s mut CommandQueue, PhantomData<fn() -> T> */ }
impl<T: Component> EnableCommands<'_, T> {
    pub fn set(&mut self, entity: Entity, on: bool);
    pub fn set_by_id(&mut self, id: EntityId, on: bool);   // resolved at apply; a dead id is a no-op
}
impl SystemMeta { pub fn declare_deferred_write(&mut self, id: ComponentId); } // for hand-written impls, in initialize

// K3
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum AmbiguityDetection { #[default] Off, Collect, Warn, Error }
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AmbiguityClass { WorldExclusive, Data, Deferred, DeferredOpen }
#[derive(Clone, Debug)]
pub struct AmbiguousPair {
    pub first: &'static str, pub second: &'static str,  // first = lower post-topological index
    pub first_index: u16, pub second_index: u16,        // transient: valid only for this Schedule (GpuBarrierEdge O2 rule)
    pub class: AmbiguityClass,
    pub items: Box<[ConflictItem]>,                      // kind (Component|Resource|NonSend|EnableTag), id, name, Rw per side
}
impl Schedule { pub fn ambiguities(&self) -> Option<&[AmbiguousPair]>; pub fn allowed_ambiguities(&self) -> Option<&[AllowedPair]>; }
impl ScheduleBuilder {
    pub fn set_ambiguity_detection(&mut self, level: AmbiguityDetection) -> &mut Self;
    pub fn allow_ambiguous_component<C: Component>(&mut self, reason: &'static str) -> &mut Self;
    pub fn allow_ambiguous_resource<R: Resource>(&mut self, reason: &'static str) -> &mut Self;
}
impl SystemConfig<'_> {
    pub fn ambiguous_with(self, other: SystemKey, reason: &'static str) -> Self;
    pub fn ambiguous_with_set<S: SystemSet>(self, set: S, reason: &'static str) -> Self;
}
impl ConfigureSet<'_> { pub fn ambiguous_with_set<S: SystemSet>(self, set: S, reason: &'static str) -> Self; }
impl App {
    pub fn set_ambiguity_detection(&mut self, level: AmbiguityDetection) -> &mut Self;  // stored; applied to both builders at finish
    pub fn ambiguities(&self, schedule: CoreSchedule) -> Option<&[AmbiguousPair]>;
}
```

## Critical-path algorithms

| Operation | Steps | Complexity | Cache behaviour | Branches / SIMD |
|---|---|---|---|---|
| K1a window (every wave) | pop k entries (unchanged) → set k bits → per word: take and zero, then `tzcnt` loop | O(k + ⌈n/64⌉) | the bitset shares lines of `ExecutorScratch` that are already hot; sequential | one loop branch per set bit; no SIMD needed (n = 36 → 1 word) |
| K1b send | lane = `state.lane` (a field load; the TLS read is gone) | O(1) | the same lane line as today | one branch fewer |
| K3 build (`Collect`) | reach sweep, then n²/2 predicate calls of ≤ 30 word ANDs each | O(E·n/64 + n²·30) | transient `reach` of n²/8 B | cold; runs once |

## Multithreading model

- **K1a:** the new state is dispatcher-owned, like `running`. No new atomics; the Acquire on `pending` and the Relaxed `fetch_sub` are unchanged. Phase 2 runs under the same exclusive `&mut EcsMaster` that SCH7 already grants.
- **K1b:** each lane has one writer, the system instance that owns the lane. The scheduler never runs one instance on two threads. The swap runs under `&mut EventDispatcher` after the join, with the same `AcqRel` swap on `write_len` (`:637`). The dispatcher lane is written only on the dispatcher thread, because worker calls to `send_event` are refused (U-21).
- **K2/K3:** init and build are single-threaded (ALLOC2). Nothing is shared at runtime. `Send`/`Sync` do not change: `OrderMeta` holds only POD and boxed slices.

## Triage of the 33 DATA pairs (hwrt `taa_armed` host)

**Rule.** A pair gets an edge if its order changes this frame's image or simulation state. Otherwise it is allow-listed with a reason that the cut verifies with one grep.

| Group | Pairs | Item | Verdict | Action |
|---|---|---|---|---|
| A | the 5 light gates among themselves (`sync_{ssao,sv0,cluster,csm,punctual}_light_gate`): 10 | `LightingConfig` W/W, `LightTableDirty` W/W | benign: each gate writes only its own `_armed` field; the dirty flag is only ever set true | allow: new `LightHeaderWriters` set, `ambiguous_with_set(self)`. The cut verifies that no gate reads another gate's field |
| B | gates × `select_lighting_cull`: 5 | `LightingConfig` W/W | benign if the fields are disjoint (the cull is declared `ResMut` even in Manual mode, `light_policy.rs:202-207`) | the cull joins `LightHeaderWriters`. If it touches an `_armed` field, it gets an edge instead |
| C | `collect_lights` × gates {ssao, csm, punctual}: 3 | `LightingConfig` R/W | **image:** the collect packs the bits the gates publish; the frame-0 header-leads-host artefact is documented at `boyko_app/tests/taa_jitter_eval.rs:522-529` | edge: `.before_set(LightCollectSet)`, as sv0 and cluster already have (`boyko_app/src/plugins.rs:737,761`) |
| D | `resolve_shadow_atlas` × {ssao, sv0, cluster, csm}: 4 | `LightTableDirty` W/W | benign: set-only flag | the atlas joins `LightHeaderWriters` |
| D′ | `resolve_shadow_atlas` × punctual gate: 1 | + `ResolvedShadowAtlas` W/R | **image:** the gate reads `mode_word` (`plugins.rs:706-711`) | edge: atlas → gate |
| E | `light_reconcile` × {`resolve_shadow_atlas`, `select_lighting_cull`, `resolve_csm_cascades`}: 3 | Point/Spot/Directional W/R | **image/sim:** consumers must see this frame's reconciled lights | edge: a reconcile set before the three |
| F | `resolve_active_camera` × {`resolve_shadow_atlas`, `resolve_csm_cascades`}: 2 | `ViewUniform` W/R | **image:** today the atlas ranks with the previous frame's view | edge: `.after_set(CameraSet::Resolve)` |
| G | `sync_csm_light_gate` × `resolve_csm_cascades`: 1 | `ResolvedCsm` R/W | **image** | edge: `CsmResolveSet` → csm gate |
| H | `particle_tick_emitters` × `particle_pack_effects`: 1 | `ParticleClock` W/R | **image** when particles are on | edge: `ParticleTickSet` → pack |
| I | fixture: `snap_apply` × `drive_{camera,moving_caster}_motion` (2), and those two with each other (1) | `Transform` | the snap pairs are **image** in motion modes; the driver × driver pair writes disjoint entities, a filter false positive (Bevy #11796) | not shipped. The fixture orders its drivers before a nameable snap set and allow-lists the driver pair |

**Totals:** 11 pairs get edges (9 edge declarations), 19 are allowed by one set declaration, and 3 belong to the fixture.

**New pairs K2 exposes** (pinned at K3b, decided here):
- `visibility_sync` × {`gather_mesh_draws`, `gather_shadow_casters`, `sync_gpu_3d_instances`, `sync_instance_model_cols`, `sync_prev_instance_model_cols` if it filters the tag}: `DeferredOpen(RenderEnabled)`. **Image; F1 itself.** Edge: a new `boyko_scene` `SceneSet::Visibility`, with the packs and gathers `.after_set` it.
- `visibility_sync` × `validate_asset_refs`: both write `RenderEnabled`. Edge: visibility → validate, so that a freed asset's disable wins.
- `snap_apply` and any hand-written `System` with the default `has_deferred`: expected false positives. They are closed by migrating to `EnableCommands<T>` or by declaring.

**World-exclusive pairs that are image-relevant:** `propagate_transforms` × the model/instance packs. Diag §6 #2 measured the packs running before propagation, which contradicts the add-order pin documented at `plugins.rs:46-53`. Edge: the packs `.after_set(CameraSet::Resolve)`. `propagate_transforms` is a member of that set (`boyko_scene/src/camera_plugin.rs:69`).

## Commit sequence

Each commit is green at the workspace level (`cargo test --workspace --all-targets --no-fail-fast`, clippy `-D warnings`). K1 is two commits because D-E0 and D-E20 have different red-first tests and different receipts (MQ-18 vs MQ-21); fused, a red could not be bisected to one of them.

| # | Commit | Files | Gate (red-first / mutation) | What moves |
|---|---|---|---|---|
| K1a | ordered apply window | `schedule/schedule.rs:12-33` (doc), `:141-169` (may_defer doc: KC-35 must keep ascending order), `:881-940`, `:1689-1804` (guard doc); `schedule/executor_scratch.rs:424-429, 542, 579, 612, 786-794` | (1) unit `apply_window_applies_in_index_order_not_pop_order`: push [3,1,2] into the channel and set their `running` bits. Expected log [1,2,3]. **Red on the parent, deterministically.** (2) `tests/k1_apply_order_w_sweep.rs`: see below. (3) Mutation: restore pop-order apply → (1) and (2) go red. EM2′-K test unchanged. UG-08 Miri on the schedule unit tests. | apply order and everything listed in D1; UG-15 leg (2) body `Schedule::run` dispatch (named in D-E0's row). No golden move expected (`taa_armed` was stable across 34 apply orders). A pin that does move was timing-dependent: re-measure 3× before any bless. |
| K1b | writer-keyed lanes | `system/params/event_writer.rs:50-63, 131-133, 162-164, 200-206`; `events/event_buffer.rs:346, 418` (lane growth at init); `events/event_dispatcher.rs:285-292`; `system/params/commands.rs:344-352` (doc); `tests/event_send_from_worker.rs` | (1) unit: two writers on one thread, B sends first. Flattened order must be A's block, then B's. **Red today:** one shared lane interleaves them. (2) D-E20's refusal test: 0 refused at every W. (3) worker `send_event` → `Err`. Mutation: restore TLS routing → red. The W sweep also checks event order. | event order; refusals; memory per type = (writers + 1) × cap (UG-20) |
| K2a | enable reads + dispatcher-solo declared access + predicate | `iters/query/filter_enable.rs:181-205, 357-362`; `system/filtered_access_set.rs:122-135, 217-274, 301-304, 317-319`; `system/params/nonsend_res.rs:98-110`, `nonsend_resmut.rs:75`; `system/system_meta.rs` (field + accessors); `system/access.rs:113-123` (doc); new `schedule/order_conflict.rs` | (1) a `Query<&M, Enabled<R>>` system conflicts with a probe that writes R. **Red today.** (2) a `NonSendRes` + `Query<&C>` system keeps a declared read of C. Mutation: drop the OR under `universal` → red. (3) `#[should_panic]` on the finalize premise assert. | Access bits of enable readers; no dispatch change (D5 argument); `SystemMeta` stays 256 B |
| K2b | deferred declarations | new `system/params/enable_commands.rs`, `commands/enable_by_id_command.rs`; `commands.rs:405-415`; `order_conflict.rs` (Deferred and Open classes) | `diagnosis_pair_is_a_deferred_conflict`: V = `Commands` + a custom command toggling R; G = concurrent `Enabled<R>` reader; X = G + `NonSendRes`. Asserts: V×G = `DeferredOpen[R]`; V×X = `DeferredOpen[R]` (named, not merely exclusive); V′ (`EnableCommands<R>`) × G = `Deferred[R]`; V″ (`EnableCommands<Other>`) × G = none. **Red before K2** (V×G is none). Mutations: remove the open rule → red; remove the declaration in `EnableCommands::init_access` → V′×G none → red. | new param; `OrderMeta` allocated at init |
| K3a | analysis + settings + allow-lists + API | new `schedule/ambiguity.rs`; `schedule_builder.rs:101-162` (fields, `new`), after `:654` (the pass), `:733-751` (literal), `:1084-1172` (error variant); `system_config.rs` (after `:135`); `system_descriptor.rs:39-86`; `schedule.rs` (trailing field + getters); `app/app.rs:125-235, 583-631`; `boyko_log` codes plus `docs/diagnostics/<code>.md` pages | proptest: `reach` equals BFS on random DAGs; a pair ordered only through a set edge is not reported; set-level allow; empty reason → build error; `Error` → `Err` listing the pairs; `Off` → `ambiguities()` is `None` (and a `cfg(test)` counter shows the pass never ran); planted canary: an unordered writer of R plus a reader gives exactly one `Data` pair | `Schedule` +8 B, trailing; no frame change |
| K3b | shipped-host ratchet | new `boyko_app/tests/schedule_ambiguity_ratchet.rs`; new `boyko_app/tests/ambiguity_pins/{sw,hwrt}_{main,fixed}.txt`; `boyko_app/src/plugins.rs:379-425` (environment variable) | Table-driven: `App::new()` + `EnginePlugins::window("ratchet", 64, 64)`, set `Collect`, `finish()` (no device needed, as in `particle_host_reachable.rs:203`); compare the Main and Fixed pins. Run both `cargo test -p boyko-app --test schedule_ambiguity_ratchet` and the same with `--features hwrt`. It must print `running N tests` with N > 0. Red controls: (a) delete `.after(propagate)` at `camera_plugin.rs:73` → new `WorldExclusive` pair → red; (b) an in-file canary composition adds a writer of `ViewUniform` → the test asserts the diff is exactly that one pair | test and pin files only |
| C1… | consumer commits (after K3b) | render/scene plugins | each deletes its pin lines; the ratchet stays green | C1: `LightHeaderWriters` allow (19 lines gone, golden-neutral). C2…: one edge group each (C, D′, E, F, G, H, the visibility set, the propagation → packs edge). Each measures its golden moves. The F1 edge gives both software and hwrt TAA pins frame-0 meshes → **owner re-bless**. Separately: `visibility_sync` migrates to `EnableCommands<RenderEnabled>`, which is golden-neutral (same commands, same queue order) and changes its class from `DeferredOpen` to `Deferred`. |

**`k1_apply_order_w_sweep`** (the D-E0 test, made stricter):
- **Setup:** 8 mutually non-conflicting systems `s0..s7`, registered in that order.
- **Each system, in each frame, by frame phase:**
  - spawn 4 × (`Tag{sys, ord}`, `Payload`) and enable `Flag` on even `ord`;
  - insert `ExtraA` (even `sys`) or `ExtraB` (odd `sys`) on `ord == 1`, which creates an archetype whose `ArchetypeId` depends on apply order; despawn `ord == 3`;
  - disable `Flag` on `ord == 0`;
  - send 3 events `(sys, seq)` through `EventWriter`.
- **Stress:** a deterministic busy-wait `((frame·5 + sys·3) mod 8) × 200 µs`, so completion order permutes frame by frame at W ≥ 2.
- **Runs:** W ∈ {1, 2, 4, 8, 16} × 48 frames, 3 runs at W = 8 and W = 16. Wall time is under 2 s.
- **Expected values are analytic, not taken from the W = 1 run:**
  - rows ascending by (sys, ord);
  - hook log ascending;
  - `ArchetypeId` creation order ExtraA before ExtraB (s0 applies first);
  - enable bits in row order;
  - events s0…s7 in blocks of 3;
  - the sorted set of live ids equal across all runs.
- **Not asserted:** which id carries which `Tag` (D3).
- **Anti-vacuity:** each system records a ticket at the end of its body. At W ≥ 2, fewer than 2 distinct end orders across the frames fails the test as "stress did not perturb". A vacuous run can therefore never pass.

## Costs

| Item | Off / release frame cost | Build cost | Memory |
|---|---|---|---|
| K1a | per window +k bit sets, ⌈n/64⌉ word take/zero, k `tzcnt`. Main n = 36 → 1 word; about 25 windows per frame, so an estimated 0.05–0.15 µs/frame. n = 1024 → 16 words per window. Receipt: MQ-18 (`ke17_apply_window`, `phase9_scheduler`, before/after, quiet window) | 0 | 0 (reuses the dead field) |
| K1b | −1 TLS read per send; flatten over (writers + 1) lanes instead of (W + 1) = 17 on this box | lane allocation at init | per type: (writers + 1) × cap × `size_of::<E>()` (UG-20, MQ-21) |
| K2 | 0 | +192 B transient per init; about 10 × 330 B `OrderMeta` | `SystemMeta` stays 256 B |
| K3 | `Off`: one predicted branch in `try_build`; `Schedule` +8 B trailing (`None`); no field is read on the run path | `Collect`: n = 36 ≈ 630 pairs × ≤ 30 word ops, about 10–20 µs; n = 1024 ≈ 5–15 ms and 128 KB transient `reach` | the report: pairs × (≈ 56 B + items) |

## What this cannot claim

- **K1:**
  - Entity id values (D3).
  - The number of Fixed steps per frame, which depends on pacing (H-11).
  - Reductions inside `par_iter` chunks (KC-37 (f)).
  - Hook order within one structural op, which follows mint order (D-E21).
  - Determinism across machines is assumed for one binary, not proven here.
  - Event order is registration order, not causal order.
  - If KC-35's split window is ever built, it must keep D1.
- **K2:**
  - Untyped structural deferred effects on archetypal components.
  - `Without<T>` is not recorded.
  - What a world-exclusive system does.
  - GPU device-column hazards, which belong to `gpu_barrier_inputs`.
  - A hand-written `declare_deferred_write` is trusted.
- **K3:**
  - False positives on entity sets that filters keep disjoint (Bevy #11796).
  - Only the engine compositions are pinned, not games.
  - Reason strings are trusted text.
  - Here an ambiguity is a **deterministic order that an unrelated edit can flip**, not timing nondeterminism. The ratchet stops silent reorders like the one behind F1; it does not prove any order correct.

## Validation

- **`debug_assert!`s:**
  - After phase 1: `apply_window ⊆ running` and `apply_window.count_ones() == target`.
  - After phase 2: the window is all zero.
  - `reach[i]` never contains any `j ≤ i`.
  - No non-universal system writes a bitset id (D5).
  - `SystemMeta` size stays 256 (the const pin).
- **Benches:** MQ-18 and MQ-21, record-only. A new `schedule_build_ambiguity` bench (n = 36 host-shaped, n = 1024 synthetic), record-only.
- **Property tests:** reachability against BFS; the pair predicate is symmetric.
- **Miri (UG-08):** schedule unit tests, and K1b's event buffer.

## Open questions

1. **"The headless host."** The tree has no separate composition for it: the `*_headless.rs` tests compose `EnginePlugins::window` and drive `update()`, so their schedules are identical to the software row. The ratchet is table-driven; if a distinct composition appears, it becomes another row.
2. **Owner call:** approve the re-bless that the F1 visibility edge causes. Both software and hwrt TAA pins gain frame-0 meshes.
3. **Unified-plan bookkeeping:** K1a/K1b satisfy D-E0/D-E20 ahead of B1–B3. UG-15 attributed mode is then not available, so they run strict with the named body, plus direct MQ-18/MQ-21 receipts.
4. **Widening the open rule** beyond enable tags to structural effects. Deferred until K3b's pins show how much noise it would add.
5. **The world-exclusive systems** (`propagate_transforms`, the seed closure). Converting them to declared-access forms would shrink the ~59 counted pairs. That is a performance and design question outside this lane.

**Checklist, N/A items:**
- Cache-line padding against false sharing: no new cross-thread state is added.
- Loom: no new atomics.
- Drop order: `OrderMeta` and `ScheduleAnalysis` are plain owned boxes, and the lifetime of `ApplyDrainGuard` is unchanged.
- Generation checks: `set_by_id` resolves the live `Entity` at apply time and does nothing if it is dead, the same contract as `SetRenderEnabledById`. ⚠ **Superseded 2026-09-21 (rungs A9 / A9b)**: that contract is hazard H-06 (see the note on D7 above); the generation is captured at enqueue, never re-resolved at apply.