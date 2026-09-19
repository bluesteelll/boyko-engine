# Architecture: kernel lane K, rev 2 (deterministic apply, access completeness, ambiguity detection)

Tree: `D:/wt/joltab` @ `47c5dabd`. Every `file:line` is from that tree unless marked *(lighttable)*, which means the `order_diag.md` run on `D:/wt/lighttable` @ `1c31aeac`. Paths without a crate prefix are under `crates/boyko_ecs/src/ecs/core/`.

## Changes from rev 1

| Remark | What rev 2 does | Where |
|---|---|---|
| **C1** (blocking) | Every read route of the enable column now declares a read: `Enabled<T>`, `Disabled<T>`, `IsEnabled<T>`, plus runtime terms. A query can carry runtime terms only if its filter contains `DynEnableAny` (reads every enable tag, the mirror of the open writer) or `DynEnableIn<S>` (reads exactly `S`, checked at runtime). This is enforced at compile time. A new census names every runtime-term call site; there are 0 in production code. K2a test (1) covers every static route. The pairs K2 exposes are re-derived with the `IsEnabled` readers and the `Commands` holders: 41 on hwrt Main, 37 on software Main, 0 on either Fixed row. | D5, "K2 exposure", K2a/K2c |
| **W1(a)** | Open-ness is decided per parameter, inside the tuple `init_access` forwarder. A deferring parameter that records no declaration of its own makes the system open. | D7 |
| **W1(b)** | A `GpuCompute` system is classified `Unknown(GpuUndeclared)` unless it calls `SystemMeta::declare_view`. Debug builds check the declaration on every `WorldView`/token access. | D6b, D10 |
| **W2** | The determinism obligation is stated over the whole frame and written into the `may_defer` doc. The W-sweep gains case `split_retire_guard`, which includes a reference model showing that a split retire would turn it red. | D1, K1a |
| **W3** | Lane 0 is permanently the dispatcher lane and is flattened last. `send_event` takes `&mut self`, so there is no thread routing left. A builder init scope mints writer lanes; everything else gets lane 0. A lane-0 run guard is added. New test covers growth. | D2a–D2e |
| **W4** | New CI step running the ratchet with `--features hwrt`, plus a CLAUDE.md leg. The file edits are named. | D14, K3b |
| **W5** | The baseline is frozen in the test and protected by a digest. Fixed pairs are recorded in `RESOLVED` (append-only). Renames go through `ALIASES` with bijection and liveness checks. A new pair has to be fixed with an edge or an in-code allow, and a census counts the in-code allows. | D13, D14 |
| **O1** | Expected rows come from a swap-remove model, not from a W=1 run. | K1a sweep |
| **O2** | The should-panic test is `#[cfg(debug_assertions)]`. | K2a |
| **O3** | Physics Fixed rows are added in `boyko_physics/tests`. | D14, K3c |
| **O4** | Declaration-order apply is documented. Debug assert: a target that was not live at apply and becomes live with the same generation was reserved but not yet spawned. | D8b |
| **O5** | `boyko_ui` id-sorted order is added to "cannot claim". | Limits |
| Critic's open questions 1–4 | Answered from the tree. | "Answers" |
| Rev 1 open questions 1, 3–5 | Settled by the orchestrator's rulings. Q2 (the re-bless) is dropped from this design. | Context |
| **Self-found (a)** | Rev 1's sweep assertion "sorted set of live ids equal across runs" is false under D3. A system that despawns its own `ord==3` entity frees an id whose identity depends on claim interleaving. It is replaced by equality of `next_fresh`, the free-stack size and the live count. | D3, K1a |
| **Self-found (b)** | `EventDispatcher::send_event(&self)` and `send(&self, lane, …)` can be reached from two threads at once, because `EcsMaster: Sync` (`ecs_master.rs:1287-1288`) and `EventDispatcher: Sync` (`event_dispatcher.rs:559`). Two unattached threads both route to lane 0 (`tls.rs:410-411`), which is a data race on one lane's `UnsafeCell` from safe code. Rev 2 closes it with `&mut self`. | D2c |
| **Self-found (c)** | A writer state initialized outside a builder (lane 0) and later added to a builder could run on a worker. A release-active guard in `EventWriter::get_param` now catches this. | D2e |

## Goal

- **K1.** Every side effect that becomes visible in an apply window is applied in an order fixed by the build, and the per-frame sequence of applies is a function of `(build, run-condition results, Fixed step count)`. It does not depend on worker count W, completion timing or the machine. Cost is O(k + ⌈n/64⌉) per window with no allocation. Event order is writer registration order, and per-writer capacity means refusals do not depend on W.
- **K2.** Every way a system touches the enable column is visible to analysis. Reads come from `Enabled<T>`, `Disabled<T>`, `IsEnabled<T>` and runtime terms. Writes come from deferred toggles, declared per parameter. The declared views of dispatcher-solo and `GpuCompute` systems are also visible. None of this changes a single dispatch decision.
- **K3.** At build, the schedule computes the pairs whose access conflicts but which have no ordering path between them. The level setting is `Off` by default and costs nothing when off. Allow-lists require a reason and are counted. A ratchet over the shipped compositions can only lose pairs.

## Context and constraints

- **Unified plan:**
  - K1a is KC-36/D-E0 (`docs/unification/UNIFIED-SYSTEM-PLAN-01-KERNEL-CONTRACT.md:188`, `02-ORDER-OF-WORK.md:161`).
  - K1b is D-E20 (`02:181`).
  - Landing both ahead of B1–B3 is **approved** (ruling 3). They run strict with the named `Schedule::run` body, plus direct MQ-18/MQ-21 receipts.
- **Invariants kept:**
  - SCH6 (`pending == 0` at frame end).
  - SCH7 gate `pending > 0 && (pending == running || running == 0)` (`schedule/schedule.rs:778`).
  - EM2′-K (`schedule.rs:159-168`, `:786-792`).
  - ADG1 (`schedule.rs:1691-1698`).
  - `SystemMeta` == 256 B (`system/system_meta.rs:63-66`).
  - `Access` == 192 B (`system/access.rs:61`).
  - `EventWriterState` == 24 B (`system/params/event_writer.rs:286-290`).
  - `EventTypeSlot` == 64 B release (`events/event_dispatcher.rs:94-95`).
  - `EventBuffer` cache-line asserts (`events/event_buffer.rs:261-276`).
  - The `Schedule` hot prefix is unchanged (`schedule.rs:193-199`).
- **Owner rules:**
  - Replays play identically at any W and on any machine, without relying on entity ids (U-20, Q-9).
  - Modding is optional and zero-overhead when unused.
  - No hot-path allocation.
  - Principle 0.
  - Every gate can fail.
  - Bugs before features.
- **Rulings folded in:**
  - Headless uses the same composition, with table-driven rows.
  - Widening the open rule to structural effects is deferred.
  - World-exclusive conversions are out of this lane.
  - The F1 re-bless is measured in the light-table lane's R4, not here.
- **Measured** *(lighttable, `order_diag.md` §1, §6)*:
  - The wave partition and dispatch order are identical across frames, runs and W ∈ {1, 3, 16}.
  - Apply order inside a wave took 34 distinct values in 34 frames at W = 3 and 16, and 1 at W = 1.
  - hwrt `taa_armed` has 236 ambiguous pairs: 203 EXCL and 33 DATA.

## K1: deterministic application

### D1. Two-phase apply window in ascending index (K1a = KC-36), kept from rev 1

- **What.** `apply_window_drain` (`schedule.rs:881-940`) is split into two phases.
  - **Phase 1** pops every `target` completion into the window bitset. No user code runs, so ADG1 holds exactly (`drained == target`).
  - **Phase 2** walks the bitset word by word, ascending, with `tzcnt`. For each system it runs today's per-system body unchanged: clear `running`, `apply`, `drain_deferred_hook_queue`, insert into `completed`, decrement successors (`:890-935`).
  - `ApplyDrainGuard` lives across phase 2, and its `Drop` still performs the single `fetch_sub(target)`.
  - The bitset is the dead field `ExecutorScratch::ready_scratch` (`schedule/executor_scratch.rs:424-429`), renamed `apply_window`. It is only cleared (`:612`) and read by nobody, so there is no size change and no allocation.
- **Why this is deterministic.**
  - Under the full barrier a window holds exactly the systems dispatched since the last window.
  - `try_dispatch_ready` scans `0..n` with no W term (`schedule.rs:1288-1341`), and the EXC2 `continue`/`break` rules (`:1309-1335`) are functions of state, not timing.
  - So window membership is fixed by the build and the run-condition results, and only the pop order was timing. Sorting it removes the dependence.
  - Everything inside `apply` inherits the order: command order, table-row order of spawns and migrations, `ArchetypeId` creation, dense and group slots, free-stack push order, hooks and observers, enable-bit writes, and lane-0 event sends.
- **Frame-level obligation (W2), written verbatim into the `may_defer` doc at `schedule.rs:141-169`:**
  > *KC-36 frame obligation.* For a fixed build, fixed run-condition results and a fixed Fixed step count, the sequence of `apply` calls in one frame, as system indices across all windows, is a function of the build alone. The full barrier plus ascending order inside a window is what guarantees it today. A split retire (KE17 step 2, lines 147-153) must keep the **frame** sequence, not only the order inside each window. Releasing a successor early must not let completion timing decide whether a deferring system applies in the same window as another deferring system or in a later one. Gate: `k1_apply_order_w_sweep::split_retire_guard`. KE17 must show that case red against its first naive split before landing.
- **Rejected:**
  - Sorting the popped `Vec`: O(k log k) and a second buffer.
  - Iterating `running.ones()`: wrong under the `running == 0` arm.
  - Recording dispatch order: couples the drain to dispatch internals.
  - Bevy-style separate sync points: our barrier is already per wave.

### D2. Writer-keyed lanes (K1b = D-E20)

- **D2 core (kept).**
  - `EventWriterState.thread_count` (`event_writer.rs:50-63`) becomes `lane: u32`, fixed at `init_state`. The state stays 24 B.
  - `send`/`send_many` read `state.lane`. The TLS routing at `:131-133` and `:162-164` is deleted.
  - Per-lane capacity is now per writer, so refusals do not depend on W (H-02).
  - A lane has exactly one writer: `EventWriter` is `&'s mut`, and the scheduler never runs one instance on two threads.
- **D2a. Lane identity (W3). Decision: lane 0 is the dispatcher lane from `preregister` onward, and the flatten visits lanes `1..L` then lane 0.**
  - Why a fixed index rather than a per-type sentinel resolved at send:
    1. Growth appends at the end, so no existing lane moves. Pending events of initialized writers and of the dispatcher lane stay where they are, and every cached index stays valid.
    2. A sentinel resolved as "last lane" at send would, on growth, turn the old last lane (holding dispatcher events) into a writer lane. Growth would then have to move pending events, which is extra work and a new class of bug.
    3. Lane 0 exists from `preregister`, so dispatcher-side sends are valid before any writer exists.
  - The flatten walk (`event_dispatcher.rs:631-637`) becomes `let (first, rest) = lanes.split_at_mut(1); for lane in rest.iter_mut().chain(first)`. This keeps rev 1's "dispatcher lane last" semantics.
- **D2b. Builder context (critic Q3).**
  - `EventDispatcher` gains `writer_lane_scope: bool`, placed in padding after `default_thread_count` (`event_dispatcher.rs:144`).
  - `ScheduleBuilder::try_build` Step 1 (`schedule/schedule_builder.rs:368-425`, the system and condition init loops) holds an RAII `WriterLaneScope` guard over `world.events_mut()` that sets the flag and clears it on drop, including during unwind.
  - `EventWriter::init_state` (`event_writer.rs:194-211`) claims a writer lane iff the flag is set. Otherwise it takes lane 0.
  - Everything initialized outside a builder therefore gets lane 0:
    - `run_system` / `run_cached_system` (`ecs_master/system_api.rs:41,146`);
    - conditions run via `run_condition` (`:230`);
    - the lighting seed's cached systems (`boyko_render/src/light_system.rs:842-845`);
    - observer runners that call `run_system`;
    - App startup closures (`app/app.rs:625-629`).
  - All of these execute under `&mut EcsMaster` on the dispatcher thread.
- **D2c. Dispatcher-side sends require `&mut` (W3 "unattached threads route to it" + self-found b).**
  - `EventDispatcher::send_event(&mut self, event)` writes lane 0, with no TLS.
  - `EventDispatcher::send`/`send_many(&mut self, lane, …)` become `&mut`; tests and benches only.
  - `EcsMaster::send_event(&self, thread_index, event)` (`ecs_master/event_api.rs:50-52`) becomes `EcsMaster::send_event(&mut self, event)`. It has no non-test callers.
  - `SendEventCommand` apply (`commands/send_event_command.rs:61`) switches to `world.events_mut()`.
  - Result: there is no "unattached thread" case. Every non-writer sender holds `&mut`, so lane 0 has one writer by borrow-checker proof.
  - `boyko_threadpool::current_worker_id_or_dispatcher_lane` (`tls.rs:396-415`) loses its event callers. It is kept (refactor-last), with its doc rewritten.
- **D2d. Lane allocation and growth (critic Q4).**
  - At `preregister`, `EventBuffer::new` allocates `cfg.thread_count` = R lanes (`event_buffer.rs:287-333`). Lane 0 is the dispatcher lane, and lanes `1..R` are reserved for writers.
  - The engine-default path passes R = 1: `preregister_event_default` (`event_api.rs:38-42`) with `default_thread_count = 1` (`ecs_master.rs:429,476`).
  - `_pad_line1` (`event_buffer.rs:233`) is repurposed as `claimed_lanes: u32`, starting at 1, so the layout asserts at `:261-276` hold.
  - `claim_writer_lane(&mut self) -> u32` returns the next reserved lane, or grows. Growth reallocates `lanes` (moving the pairs, so pending events survive) and `reader_buf` (moving the initialized prefix `[..reader_len]`).
  - Growth happens only inside the builder scope, under `&mut EcsMaster`. No reader item and no send can be live, because readers re-derive `reader_buf` on every call (`system/params/event_reader.rs:152`).
  - Ceiling: `MAX_EVENT_THREADS = 65` (`constants.rs:418`). Past it, a cold build-time panic names the event type.
  - `EventTypeSlot.thread_count` (a diagnostic mirror) is updated on growth.
  - Memory per type = `max(R, writers + 1) × cap × size_of::<E>()` for lanes, and the same for `reader_buf` (UG-20). It no longer depends on W unless the caller passes W in R.
- **D2e. Lane-0 run guard (self-found c).**
  - `EventWriter::get_param` runs `if state.lane == DISPATCHER_LANE { lane0_run_guard() }`.
  - The guard is `#[cold] #[inline(never)]` and panics if `current_worker_id()` is a pool worker id (`tls.rs:392-394`), in release as well.
  - Reason: a system first initialized outside a builder (for example via `run_cached_system(&mut sys)`) and then moved into a builder keeps lane 0 (FS1 idempotence, `system/function_system.rs:188-190`). Two such systems running concurrently on workers would race on lane 0. With the guard, lane-0 writers execute only on the single dispatcher or unattached thread.
  - Cost: one predicted compare per writer run. Builder-minted writers never take the branch.
- **Rejected:**
  - Worker lanes with a (writer, seq) sort at swap: +4–8 B per event and a sort per swap (MQ-21's overturn form).
  - Lanes keyed by post-topological index: needs a unified Main+Fixed index space.
  - All sends through commands: +18 ns per event, and every send serialized into the apply window.
  - Bevy's model: `MessageWriter` holds `ResMut<Messages<T>>`, so writers of one type never run in parallel and ambiguity detection sees them as W/W conflicts. We keep parallel sends and get build-order determinism from lane identity.

### D3. Entity ids are not deterministic (ruling Q-9), kept, with a corrected corollary

- Ids are claimed on the worker at enqueue (`system/params/commands.rs:169-173`), and that stays timing-dependent.
- **Deterministic:**
  - the number of claims per window;
  - the free-stack size;
  - `next_fresh`, since fresh mints = max(0, claims − stack size);
  - the (system, ordinal) → row mapping;
  - live counts.
- **Not deterministic:**
  - the id → (system, ordinal) mapping;
  - therefore the set of ids freed by despawning one's own entities;
  - therefore the live id set after such a despawn. Rev 1 wrongly asserted that this set was equal across runs.
- Rejected:
  - Apply-time id claim: breaks the synchronous `spawn().id()`.
  - Per-system leases (MQ-19): not requested.

### D4. Enable-bit writes need no mechanism of their own, kept

`set_enable_bit` takes `&mut self` (`ecs_master/enable_tag_api.rs:149`). The only writers are therefore apply windows, inline exclusives and setup code, and D1 orders all of them.

## K2: access completeness

### D5. Every read route of the enable column declares a read (C1)

**Static routes.** `init_access` calls `access_set.add_component_read(state.id, type_name::<Self>())`, the same form `With<T>` uses (`iters/query/filter.rs:569-573`):

| Route | Site today | Change |
|---|---|---|
| `Enabled<T>` | `iters/query/filter_enable.rs:181-205` (ENBL-ACCESS-1, no-op) | Declare the read and rewrite ENBL-ACCESS-1: the filter reads the column during the parallel phase, which is why the bit is declared. |
| `Disabled<T>` | `filter_enable.rs:357-362` | Same. |
| `IsEnabled<T>` | `iters/query/data_is_enabled.rs:170-176`; docs at `:34-36`, `:127-132` | Same. The QD1 SAFETY text is rewritten. |
| AND tuples and `Or` | forward to their elements (`filter.rs:1547` macro) | No change. |

**Runtime terms.** `Query::with_enabled` / `without_enabled` (`iters/query/query.rs:200,215`) now require the filter `F` to admit dynamic enable terms.

- New `QueryFilter` items, both with defaults:
  - `const DYN_ENABLE: DynEnableKind = None`
  - `fn dyn_enable_admits(state, id) -> bool { false }`
  - The AND-tuple macro ORs them. `Or` keeps the default.
- New file `iters/query/filter_dyn_enable.rs` with two markers:
  - `DynEnableAny`:
    - `init_access` calls `access_set.mark_enable_read_all()`, which reads **every** enable tag, including ones registered later. This is the mirror of the open writer.
    - `dyn_enable_admits` is a constant `true`.
  - `DynEnableIn<S: EnableTagSet>`:
    - `init_state(world)` resolves `S` into `[ComponentId; MAX_ENABLE_TERMS]` (`constants.rs:449` = 8). Typed tags go through tuple impls; named dynamic tags go through a user marker whose `resolve` calls `world.register_enable_tag(name)`.
    - `init_access` declares a read of each id.
    - `dyn_enable_admits` tests membership.
- Enforcement:
  - `with_enabled` begins with `const { assert!(F::DYN_ENABLE != None) }`.
  - It then runs a release-active `#[cold]` panic if `!F::dyn_enable_admits(&state.filter_state, tag.component_id())`.
  - Precedent in our tree: `assert_dense_iter_no_enable` (`query.rs:101-125`), including the `pub const fn assert_dyn_enable_admitted::<F>()` needed for a trybuild compile-fail fixture under `cargo check`.
  - Precedent in Bevy: `transmute_lens` panics when the new terms need access the query did not declare.
- Cheap enough to adopt:
  - No per-row cost.
  - `DynEnableAny`'s check const-folds away.
  - `DynEnableIn` costs at most 8 compares per term push, which happens once per view construction.
  - One new file (~250 lines), two trait items with defaults, and one macro line.
- `QueryView::with_enabled` / `without_enabled` (`query_view.rs:283,298`) need no marker. A `QueryView` is reachable only through `EcsMaster::query(&mut self)` (`ecs_master/ecs_master.rs:825,929`), meaning from world-exclusive systems (already `Unknown`) or from code outside schedules.

**Census.**
- A new root test, `tests/enable_term_census.rs`, pins every non-comment `.with_enabled(` / `.without_enabled(` site under `crates/`. Each entry is a (path, count, kind ∈ {Query, QueryView}) triple. A new site or a count change fails the test.
- Today's list:
  - Query:
    - `boyko_ecs/tests/enable_tag_step9.rs`: 4 (`:175,189,205,232`).
    - `enable_chunk_byte_identity.rs`: 9 (`:154,158,176,178,323,486,493,517,524`).
  - QueryView:
    - `enable_tag_step9.rs:280`.
    - `enable_tag_derive_step10.rs:330`.
    - `enable_chunk_byte_identity.rs:377,379,600,601`.
    - `iters/query/state.rs:1558,1568`.
    - `benches/enable_chunk_filter.rs:125,141,182`.
  - **Production sites: 0.**
- The 13 Query sites gain `DynEnableAny` in their filter type in the same commit.

**Why dispatch cannot change.**
- A conflict needs a write bit on the same id. No non-universal system can hold a component write on a bitset id: bitset ids have no pool, and `EnableCommands` records into `OrderMeta`, not `Access`.
- Universal systems already conflict with everything, and `conflicts_with(universal, empty)` is false (`access.rs:214-222`).
- `GpuCompute` has empty `Access` and is solo under EXC2 (`schedule.rs:1326`).
- The executor never reads the enable-read-all flag.
- So the dispatch sequence is byte-identical and no golden moves.
- Premise check: a `debug_assert!` in `FilteredAccessSet::finalize` (`system/filtered_access_set.rs:317-319`) that `component_writes ∩ bitset_ids == ∅` for a non-universal system, and also for any declared view.

### D6. Dispatcher-solo exclusives keep what they declare, kept

- `FilteredAccessSet` (`filtered_access_set.rs:121-135`) accumulates `declared: Access`. Every `add_*` (`:154-274`) ORs into it even after `mark_universal` (`:301-304`) has made the `combined` path a no-op (FIX-1/X2).
- NonSend params record `(NonSendResourceId, is_write)` (`params/nonsend_res.rs:98-110`, `nonsend_resmut.rs:75`).
- At `finalize`, `universal && meta.requires_dispatcher` moves the declared view into `OrderMeta` with `DECLARED_VIEW`.
- Five Main exclusives are exclusive only because of NonSend:
  - `apply_refcount_deltas`, `validate_asset_refs` (`boyko_render/src/asset_refcount.rs:89,401`);
  - `gather_shadow_casters`, `reduce_caster_bounds` (`csm_caster.rs:210,472`);
  - `gather_mesh_draws` (`mesh_draw.rs:1238`).

### D6b. `GpuCompute` declares its view or is `Unknown` (W1b)

- A `GpuCompute` system has empty `Access` by design (`boyko_render/src/gpu_system.rs:13-25,266-272`; `boyko_render/src/ui/upload.rs:293-309`). It reaches the world through `DispatcherToken::world()`, whose `WorldView` offers `resource`, `try_resource`, `get_component_raw` and `query_entities_buf` (`system/dispatcher_token.rs:198-360`), plus `nonsend_resource{,_mut}` (`:125-181`).
- **Declaration.** Called from `System::initialize(&mut self, world)`:
  ```rust
  meta.declare_view(world, |v| {
      v.read::<C>();
      v.resource::<R>();
      v.nonsend::<N>(write);
  });
  ```
  - One call is one declaration, and it sets `DECLARED_VIEW` even when the closure is empty.
  - A declared `GpuCompute` that flushes nothing must also override `has_deferred() -> false` (`system/system.rs:190`). Otherwise D7 classifies it as open.
- **Verification (debug only, no TLS).**
  - `DispatcherToken::new` (`dispatcher_token.rs:97-104`) gains a `#[cfg(debug_assertions)]` copy of the declared view: 192 B `Access` + 32 B NonSend array. The copy is taken from `self.systems[i].system.meta()` at `schedule.rs:1418-1419`, and is `None` from `run_system_once`.
  - Every `WorldView` accessor and NonSend projection then `debug_assert!`s that its id lies inside the declared view, on top of the existing M2 thread tripwire (`dispatcher_token.rs:264-274`).
  - A copy, rather than a pointer into the system's own `Box<OrderMeta>`, avoids aliasing the system's `&mut self` during `run_dispatcher`.
- **Classification:** `GpuCompute` without `DECLARED_VIEW` is `Unknown(GpuUndeclared)` and pairs with every unordered system, like world-exclusive.
- **On the host today:** zero `GpuCompute` systems are registered. There is no `.gpu()` call and no `GpuSystem` or `UiUploadSystem` registration in `crates/*/src`. The two `boyko_render` impls stay undeclared until their owners declare.

### D7. Deferred writes are declared by construction, per parameter (W1a)

- **Typed param `EnableCommands<'s, T>`** (new file `system/params/enable_commands.rs`):
  - It owns a `CommandQueue`, has `HAS_DEFERRED = true`, and its `init_access` calls `access_set.declare_deferred_write(T::component_id(), name)`, which increments `deferred_records` and sets the mask bit.
  - `set(Entity, bool)` pushes `EnableTagCommand` (`commands/enable_tag_commands.rs:45-61`).
  - `set_by_id(EntityId, bool)` pushes the new `commands/enable_by_id_command.rs`, which resolves the entity at apply and is a no-op if it is dead. This is the `SetRenderEnabledById` contract (`boyko_scene/src/visibility_sync.rs:84-106`) moved into the kernel.
- **Untyped `Commands`:** `init_access` (`params/commands.rs:405-415`) calls `access_set.mark_deferred_untyped(name)`, which sets `OPEN` and increments the records.
- **Per-parameter accounting.** The tuple forwarder (`system/params/tuple_impl.rs:110-123`) wraps each element:
  ```rust
  if <$p>::HAS_DEFERRED {
      let r = access_set.deferred_records();
      <$p>::init_access(...);
      if access_set.deferred_records() == r {
          access_set.mark_deferred_open(type_name::<$p>());
      }
  } else {
      <$p>::init_access(...);
  }
  ```
  - `HAS_DEFERRED` is a const, so non-deferring elements compile to today's code.
  - `FunctionSystem`'s `F::Param` is always a tuple (`system/function_system.rs:222-230`, per the `function_system_impls` wrapping), so the top level is covered.
  - A hand-written composite `SystemParam` counts as one parameter. There is no derive: every kernel composite is a tuple, and the only external impl is `tests/into_system_closure_inference.rs:66`.
- **Hand-written `System` impls:** open unless `has_deferred() == false`, or `initialize` calls the public `SystemMeta::declare_deferred_write`. This is trusted, like `has_deferred` itself.
- **Effective status, computed at build:**
  - `!has_deferred()` → none.
  - `OPEN` set, or `deferred_records == 0` → open.
  - Otherwise → closed with the mask.
- **Rejected:**
  - Config-level `.defers_write::<T>()`: a lying declaration drops pairs silently.
  - An apply-time TLS audit: adds a `thread_local!` against the D-M6 census and checks only exercised paths.

### D8. Open rule and metadata

- **Open rule.** An open deferrer is assumed to write every enable tag. It pairs with every reader of any enable tag (static read bits on bitset ids, or `READS_ALL_ENABLE`) and with every deferred writer, closed or open.
- **Scope stays enable tags only** (ruling 4). They are the only component class whose non-exclusive writers are exclusively apply windows.
- **Metadata:**
  - `SystemMeta.order: Option<Box<OrderMeta>>` sits in the tail padding. `zone` is the last field (`system_meta.rs:174`), the next 8-aligned offset is 248, and 248 + 8 = 256, so the pin holds.
  - `None` for plain systems (the 0% gate), following the `gpu_intent` precedent at `:140`.
  - Allocated at init for `Commands`/`EnableCommands` holders, `DynEnable*` holders, dispatcher-solo systems and declared `GpuCompute` systems.

### D8b. Order between `EnableCommands` and `Commands` within one system (O4)

- **Documented** on `EnableCommands`:
  - Params apply in declaration order (`tuple_impl.rs:156-166`), and systems apply in ascending index (D1).
  - A toggle whose target is spawned by a queue that applies **later** is a silent no-op (`ecs_master/enable_tag_api.rs:157-160`).
  - Declare `Commands` before `EnableCommands`, or use `EntityCommands::enable` on the spawning queue.
- **`debug_assert!` on a reserved-but-not-yet-spawned target.** Debug builds only, in the `EnableCommands` state, with no TLS:
  - At apply, a `set(Entity)` target that is not live and whose id is at or above `entity_master.next_fresh()` at apply time fails the assert immediately. That id was reserved in this window and not yet spawned.
  - Every other non-live `set(Entity)` target is recorded in a fixed `[Entity; 8]` plus an overflow count.
  - At the start of the next `apply`, which holds `&mut EcsMaster`, any recorded target that is now live with the **same generation** fails the assert with the declaration-order message.
  - This is exact: a stale handle can never become live again, because generations only increase on recycle, so only a reserved-then-spawned id satisfies the check.
  - `set_by_id` is not covered: it carries no generation. This is named in the limits.

### K2 exposure, re-derived (C1)

**Symbols:**
- Readers:
  - R1 `validate_asset_refs` (`asset_refcount.rs:402`, `Enabled<RenderEnabled>`)
  - R2 `gather_shadow_casters` (`csm_caster.rs:213`)
  - R3 `sync_gpu_3d_instances` (`gpu3d_system.rs:47`)
  - R4 `sync_instance_model_cols` (`instance_model.rs:111`)
  - R5 `sync_prev_instance_model_cols` (`instance_model.rs:192`, hwrt only)
  - R6 `gather_mesh_draws` (`mesh_draw.rs:1253`/`:1368`)
  - R7 `snap_apply` (`snap_interpolation.rs:126`, `Enabled<SnapInterpolation>`)
  - R8 `collect_lights` (`light_system.rs:593-596`, `IsEnabled<LightEnabled>`)
  - R9 `select_lighting_cull` (`light_policy.rs:199-200`, `IsEnabled`)
  - R10 `particle_tick_emitters` (`particle_system.rs:342`, `Enabled<EmitterActive>`)
- Open deferrers (untyped `Commands`):
  - D1 `visibility_sync` (`boyko_scene/src/visibility_sync.rs:143-151`)
  - D2 `apply_refcount_deltas` (`asset_refcount.rs:89-96`; its only command is `SyncRefGenCommand` at `:113`)
  - D3 = R1
  - D4 = R7

**Ordering edges read:**
- `plugins.rs:676-766` (`prev.before(pack)`, `casters.after(pack)`, `gather_mesh_draws.after(pack).after(snap)`)
- `camera_plugin.rs:68-73` (`visibility_sync.after(propagate)`)
- `asset_refcount.rs:484-485` (D2 → D3)
- `light_plugin.rs:119-126`
- `particle_plugin.rs:90-97`
- `render3d_plugin.rs:38`

**Result on hwrt Main: 41 `DeferredOpen` pairs.**

| Deferrer | Readers with no path | New (both sides non-exclusive) | Reclassified from EXCL |
|---|---|---|---|
| D1 | R1–R10 (10) | R3, R4, R5, R7, R8, R9, R10 (7) | R1, R2, R6 (3) |
| D2 | R2–R10 (9); R1 is ordered | – | 9 |
| D3 | R2–R10 (9) | – | 9 |
| D4 | R1–R5, R8–R10 (8); R6 is ordered | R3, R4, R5, R8, R9, R10 (6) | R1, R2 (2) |
| open × open | D1D2, D1D3, D1D4, D2D4, D3D4 (5); D2D3 is ordered | D1D4 (1) | 4 |

- Totals: hwrt 41 (14 new, 27 reclassified); software 37 (12 new, 25 reclassified).
- This is a falsifiable prediction: K3b's first `Collect` run must reproduce 41/37, or the K3b commit explains the difference pair by pair.
- **Fixed rows:**
  - EnginePlugins Fixed holds only `pack_gpu_transforms` (`plugins.rs:780-781`; `IsEnabled<SnapInterpolation>` at `gpu_transform_pack.rs:74`): 0 pairs.
  - Physics Fixed readers are `scene_sync.rs:83,145,193`, `systems.rs:142`, and the gather through `body_set.rs:106-113`. There is no deferrer in the composition (`boyko_physics/src/plugin.rs:599-781`), so 0 pairs.
  - The exposure there is for games. A Fixed spawner using `spawn_dynamic` (`bundles.rs:71-75`, a `Commands` holder that enables `Simulated`) and not ordered before the physics block head gets `DeferredOpen[Simulated]` with each of the 5 readers. K2c test (5) proves exactly that.
- **Resolution (consumer commits, after K3b):**
  - D1, D3 and D4 migrate to `EnableCommands<RenderEnabled>` / `EnableCommands<SnapInterpolation>::set_by_id`. They become closed, which removes 28 of the 41.
  - What remains:
    - (a) D1′ × RenderEnabled readers and D1′ × D3′, class `Deferred`: the F1 edge (new `SceneSet::Visibility` before the packs and gathers) and visibility → validate.
    - (b) D3′ × R2–R6, class `Deferred`: edges validate → {packs, gathers}. This encodes the documented "apply → validate → gather" contract (`asset_refcount.rs:371-378`), which has no edge today.
    - (c) D2 × R3–R5, R7–R10, class `DeferredOpen`: `ambiguous_with_set(…, "apply_refcount_deltas' only command is SyncRefGenCommand (asset_refcount.rs:113); writes no enable tag")`. D2 × R2, R6 are ordered by (b).

## K3: ambiguity detection

### D9. Build-time analysis on the post-topological DAG, kept

- The pass is `#[cold] #[inline(never)]` in the new file `schedule/ambiguity.rs`. It runs after `ConflictGraph::build` (`schedule_builder.rs:654`) and before step 10 consumes the descriptors (`:664-687`).
- Reachability is a reverse topological sweep: `reach[i] = ∪ over successors s of ({s} ∪ reach[s])`. Set edges are already expanded (`:515-525`).
- For each unordered pair: allow-list filter, then the D10 predicate.

### D10. Pair predicate (`schedule/order_conflict.rs`)

**Each system's view:**
- **Unknown:**
  - world-exclusive: universal access without `requires_dispatcher`;
  - `GpuCompute` without `DECLARED_VIEW`.
- **Declared:**
  - dispatcher-solo CPU: declared `Access` plus NonSend uses;
  - declared `GpuCompute`: the same.
- **Concurrent:** `meta.access`, whose enable-read bits come from D5.
- **Added to every view:**
  - `READS_ALL_ENABLE`;
  - deferred status: none, closed with mask, or open;
  - reads of the system's own run conditions and of any gating set's conditions.

**Classes** (strongest wins; items are listed):

| Class | Condition | Items |
|---|---|---|
| `Unknown(WorldExclusive \| GpuUndeclared)` | either side is `Unknown` | none |
| `Data` | declared R/W or W/W over components or resources; NonSend between two declared views | ids with R/W per side |
| `Deferred` | closed mask ∩ (other side's reads ∪ writes ∪ deferred mask ∪ all-if-`READS_ALL_ENABLE`) | ids |
| `DeferredOpen` | one side open and the other reads, or defer-writes, any bitset id (open × open counts) | those ids, or `EnableTag(*)` |

**Events are excluded.**
- Readers see only the post-swap flat buffer (`event_reader.rs:78-80`, `:115-154`), and the swap runs once per frame (`app.rs:713-720`).
- With D2, cross-writer order is build order regardless of execution order.

### D11. Level setting and API, kept

- `AmbiguityDetection { Off (default), Collect, Warn, Error }`, set per builder and per `App`.
- `Off` computes nothing.
- `Warn` emits one new `boyko_log` warning code per pair; the code and its `docs/diagnostics` page are reserved in K3a.
- `Error` makes `try_build` return `Err(Ambiguities)`, and `build` panics with a new B code.
- `EnginePlugins` maps `BOYKO_SCHEDULE_AMBIGUITY` (`plugins.rs:379-425`). An unknown value reports through `W3009`.
- Enforcement is the ratchet, not the default. Bevy likewise defaults to `Ignore` and enforces in CI.

### D12. Exclusive pairs, kept, extended to `GpuCompute`

- `Unknown` pairs are all counted (Bevy parity).
- Dispatcher-solo and declared `GpuCompute` pairs are analysed.
- Expected world-exclusive `Unknown` pairs: about 59. `propagate_transforms` contributes ~30 (`boyko_scene/src/propagation.rs:222`). The seed closure takes `&mut EcsMaster` (`boyko_render/src/light_plugin.rs:123`, now read, no longer assumed) and contributes ~30, minus 1 pair counted twice. K3b measures it.

### D13. Allow-lists require a reason and are counted

- Methods:
  - `SystemConfig::ambiguous_with(key, reason)`
  - `SystemConfig::ambiguous_with_set(set, reason)`
  - `ConfigureSet::ambiguous_with_set(set, reason)` (self-set = within-set pairs)
  - `ScheduleBuilder::allow_ambiguous_component::<C>(reason)`
  - `ScheduleBuilder::allow_ambiguous_resource::<R>(reason)`
- An empty reason is a build error. There is no `ambiguous_with_all`.
- Allow-lists affect diagnostics only.
- **New root census `tests/ambiguity_allow_census.rs`:**
  - Pins every non-comment call of these five methods under `crates/*/src` as a (path, reason literal) pair.
  - The reason must be a string literal, not a variable, and not a placeholder (`todo`, `tbd`, `fixme`).
  - Any new allow fails the census until it is listed in the same commit, so every allow is reviewed twice: in the code and in the census.

### D14. Ratchet (W5: can only lose pins; W4: standing hwrt leg; O3: physics rows)

**Per-row constants in the test file:**
- `FROZEN: &[(&str, &str, &str)]`
  - Canonical `(min_name, max_name, class)` lines captured at freeze, compared as a multiset (closures can share a `type_name`).
  - Protected by `FROZEN_DIGEST: u64`, an FNV-1a hash over the lines. The test recomputes it, so any edit of `FROZEN` is red unless the literal is also edited. The test doc says the digest changes only in a commit that changes the detector.
- `RESOLVED: &[usize]`
  - Append-only indices into `FROZEN` for pairs that no longer occur. Entries are unique and in range.
  - A pinned pair that disappears is red: "add #k to RESOLVED".
  - A resolved pair that comes back is red: "#k resolved earlier has returned".
- `ALIASES: &[(old, new, reason)]` for renames. Checks:
  - `old` is in `FROZEN`;
  - `new` is present in the live schedule;
  - `old` is absent from the live schedule (a real rename, not a substitution);
  - the map is a bijection.
- `ALLOWED: &[(a, b, class, reason_prefix)]`: exact both ways. It grows only together with an in-code allow, which the census also counts.

**Rule:** live ambiguous = (`FROZEN` − `RESOLVED`) after aliasing, exactly. Any pair outside that set is red. The fix is an edge or an in-code allow, never a `FROZEN` line. The comparison logic is a `#[cold]` public `ambiguity::ratchet_compare(...)`, so both crates share it.

**Rows:**
- `boyko_app/tests/schedule_ambiguity_ratchet.rs`: `App::new()` + `EnginePlugins::window("ratchet", 64, 64)`, `Collect`, `finish()`. No device is needed (precedent: `boyko_app/tests/particle_host_reachable.rs:203`). Rows (sw, Main), (sw, Fixed), and under `cfg(feature = "hwrt")` (hwrt, Main), (hwrt, Fixed).
- `boyko_physics/tests/schedule_ambiguity_ratchet_physics.rs`: a bare `ScheduleBuilder` with `Collect`. One row per public entry point: `add_physics_systems`, `_with_scene_sync`, `_colored`, `_colored_solve`, `_sdf`, `_soft`, `_soft_colored` (`boyko_physics/src/plugin.rs:204,263,289,322,356,408,446`), using the crate's default solver type.
- Generator: a `#[ignore = "generator: prints FROZEN tables for a detector commit"]` test.

**Standing legs (W4):**
- `.github/workflows/ci.yml`, job `test` (`:96-112`), gains a step after `:112`, gated `if: matrix.profile == 'debug'`. It runs `cargo test -p boyko-app --features hwrt --test schedule_ambiguity_ratchet` and fails unless the output contains `^running [1-9][0-9]* tests?$`, the same vacuous-green guard style as `:404-407`.
- `CLAUDE.md` gains, after "Leg: device-free ignored tests", a new entry "Leg: feature-gated device-free gates" with the same command and "output must read `running 5 tests`".
- The software rows and the physics rows run in the existing workspace test step.

## Data structures

```rust
// executor_scratch.rs:424-429: renamed, same type and size.
pub(crate) apply_window: FixedBitSet,        // n bits; dispatcher-owned; filled in phase 1, zeroed word by word in phase 2

// system_meta.rs: appended after `zone` (:174); offset 248; size_of stays 256 (pin :63-66)
pub(crate) order: Option<Box<OrderMeta>>,    // None = 0% gate (the gpu_intent precedent, :140)

#[repr(C)]                                   // heap; written at init, read at build and by debug token checks
pub(crate) struct OrderMeta {                // ≈ 320 B at align 32; about 10 per host
    declared: Access,                        // 192 B: declared view (dispatcher-solo / GpuCompute); valid iff DECLARED_VIEW
    deferred_writes: ComponentMask,          // 64 B: closed deferred mask (EnableCommands<T>, declare_deferred_write)
    nonsend: [(u32 /*NonSendResourceId*/, bool /*write*/); 4], // 32 B inline, no second Box (rev 1 had Box<[..]>)
    nonsend_len: u8,
    flags: u8,                               // DECLARED_VIEW | DEFERRED_OPEN | READS_ALL_ENABLE | NONSEND_OVERFLOW (⇒ treat as all NonSend)
    deferred_records: u16,                   // per-param accounting (D7)
    open_param: &'static str,                // first undeclared deferring param; report text only
}

// filtered_access_set.rs:121-135: transient at init, + OrderAccum (same fields, unboxed); moved into meta.order by finalize (:317-319)

// event_writer.rs:50-63: 24 B pin holds
lane: u32,                                   // replaces thread_count; 0 = DISPATCHER_LANE, 1.. = writer lanes (build order)

// event_buffer.rs:233: `_pad_line1: u32` → `claimed_lanes: u32` (starts at 1); line asserts :261-276 unchanged
// event_dispatcher.rs:134-145: + writer_lane_scope: bool, in padding after default_thread_count

// schedule.rs: appended after world_id (:229); hot prefix untouched
pub(crate) analysis: Option<Box<ScheduleAnalysis>>,   // None when Off
pub(crate) struct ScheduleAnalysis { pairs: Box<[AmbiguousPair]>, allowed: Box<[AllowedPair]> }

// system_descriptor.rs:39-71 (build only): ambiguous_with: Vec<(AllowTarget, &'static str)>
// ScheduleBuilder (:101-162): ambiguity: AmbiguityDetection (1 B), set_ambiguity, allowed_items (build-only Vecs)
// enable_commands.rs state: CommandQueue + #[cfg(debug_assertions)] { pending: [Entity; 8], len: u8, overflow: u16 } (D8b)
```

## Public API

```rust
// K1b
pub const DISPATCHER_LANE: u32 = 0;
impl EventDispatcher {
    pub fn send_event<E: Event>(&mut self, e: E) -> EcsResult<()>;                  // was &self + TLS
    pub fn send<E: Event>(&mut self, lane: u32, e: E) -> EcsResult<()>;             // was &self
    pub fn send_many<E: Event, I: ExactSizeIterator<Item = E>>(&mut self, lane: u32, it: I) -> EcsResult<()>;
}
impl EcsMaster { pub fn send_event<E: Event>(&mut self, e: E) -> EcsResult<()>; }  // was (&self, thread_index, e)

// K2
pub struct DynEnableAny;                                   // QueryFilter marker
pub struct DynEnableIn<S: EnableTagSet>(PhantomData<S>);   // QueryFilter marker
pub trait EnableTagSet: Send + Sync + 'static { fn resolve(world: &mut EcsMaster, out: &mut EnableTagIds); }
pub const fn assert_dyn_enable_admitted<F: QueryFilter>();
pub struct EnableCommands<'s, T: Component> { /* &'s mut CommandQueue, PhantomData<fn() -> T> */ }
impl<T: Component> EnableCommands<'_, T> {
    pub fn set(&mut self, e: Entity, on: bool);
    pub fn set_by_id(&mut self, id: EntityId, on: bool);   // resolved at apply; dead id = no-op
}
impl SystemMeta {
    pub fn declare_deferred_write(&mut self, id: ComponentId);                   // hand-written System impls
    pub fn declare_view(&mut self, world: &EcsMaster, f: impl FnOnce(&mut ViewDecl<'_>)); // GpuCompute
}
impl ViewDecl<'_> {
    pub fn read<C: Component>(&mut self);
    pub fn resource<R: Resource>(&mut self);
    pub fn nonsend<N: NonSendResource>(&mut self, write: bool);
}

// K3 (as in rev 1, with the new Unknown class)
pub enum AmbiguityDetection { #[default] Off, Collect, Warn, Error }
pub enum AmbiguityClass { Unknown(UnknownReason), Data, Deferred, DeferredOpen }
pub enum UnknownReason { WorldExclusive, GpuUndeclared }
pub struct AmbiguousPair {
    pub first: &'static str, pub second: &'static str,
    pub first_index: u16, pub second_index: u16,          // valid only for this Schedule
    pub class: AmbiguityClass, pub items: Box<[ConflictItem]>,
}
impl Schedule {
    pub fn ambiguities(&self) -> Option<&[AmbiguousPair]>;
    pub fn allowed_ambiguities(&self) -> Option<&[AllowedPair]>;
}
impl ScheduleBuilder {
    pub fn set_ambiguity_detection(&mut self, l: AmbiguityDetection) -> &mut Self;
    pub fn allow_ambiguous_component<C: Component>(&mut self, reason: &'static str) -> &mut Self;
    pub fn allow_ambiguous_resource<R: Resource>(&mut self, reason: &'static str) -> &mut Self;
}
impl SystemConfig<'_> {
    pub fn ambiguous_with(self, k: SystemKey, reason: &'static str) -> Self;
    pub fn ambiguous_with_set<S: SystemSet>(self, s: S, reason: &'static str) -> Self;
}
impl ConfigureSet<'_> { pub fn ambiguous_with_set<S: SystemSet>(self, s: S, reason: &'static str) -> Self; }
impl App {
    pub fn set_ambiguity_detection(&mut self, l: AmbiguityDetection) -> &mut Self;
    pub fn ambiguities(&self, s: CoreSchedule) -> Option<&[AmbiguousPair]>;
}
#[cold] pub fn ratchet_compare(row: &RatchetRow, live: &ScheduleAnalysis) -> Result<(), RatchetReport>;
```

## Critical-path algorithms

| Operation | Steps | Complexity | Cache | Branches / SIMD |
|---|---|---|---|---|
| K1a window (every wave) | pop k (unchanged) → set k bits → per word: take, zero, `tzcnt` loop | O(k + ⌈n/64⌉) | window bitset shares already-hot `ExecutorScratch` lines; sequential | one branch per set bit; Main n = 36 is one word |
| K1b `EventWriter::send` | lane = `state.lane` (a field load; the TLS read is gone) | O(1) | same lane line | one fewer call; `get_param` adds one predicted compare |
| K1b flatten | lanes 1..L then 0; memcpy per non-empty lane | O(L + events) | sequential per lane | L = max(R, writers + 1) instead of W + 1 = 17 |
| K2 runtime term push | `DynEnableIn` membership over ≤ 8 ids | O(8) per term push, once per view | L1 | no per-row change |
| K3 build (`Collect`) | reach sweep, then n²/2 predicate calls of ≤ 40 word ops | O(E·n/64 + n²·40) | transient `reach` of n²/8 B | cold, once per build |

## Multithreading model

- **K1a:** dispatcher-owned state; no new atomics. The Acquire on `pending` and the Relaxed `fetch_sub` are unchanged. Phase 2 runs under the SCH7 `&mut EcsMaster`.
- **K1b:**
  - Writer lanes 1..: one owning state per lane, one thread at a time.
  - Lane 0: only `&mut EventDispatcher` holders and out-of-builder states, and the D2e guard confines the latter to the dispatcher or unattached thread, of which there is one per world run.
  - Growth: only inside the builder scope under `&mut EcsMaster`.
  - Swap: `&mut EventDispatcher` after the join, with the same `AcqRel` swap on `write_len` (`event_dispatcher.rs:637`).
  - `ThreadLaneWriter`'s `Sync` SAFETY text (`event_buffer.rs:134-151`) is rewritten from "per worker" to "per lane owner", and so is `EventDispatcher`'s (`event_dispatcher.rs:535-557`).
- **K2/K3:** init and build are single-threaded (ALLOC2). The debug view copy lives on the dispatcher's stack. `Send`/`Sync` are unchanged, since `OrderMeta` is POD plus `&'static str`.
- **Proof of no new race:** no new shared mutable state is read concurrently. D2c removes an existing race (self-found b).

## Triage of the 33 DATA pairs (hwrt `taa_armed`, *lighttable*)

K3b re-measures on this tree. Rule: an edge if the order changes this frame's image or simulation state; otherwise an allow with a reason one grep verifies.

| Group | Pairs | Item | Verdict | Action |
|---|---|---|---|---|
| A | 5 gates among themselves: 10 | `LightingConfig` W/W, `LightTableDirty` W/W | Benign: each gate writes only its own fields; the dirty flag is only ever set | Allow: `LightHeaderWriters` set, `ambiguous_with_set(self)` |
| B | gates × `select_lighting_cull`: 5 | `LightingConfig` W/W | **Resolved from the tree:** the cull writes only `clusters_enabled` (Auto mode) and `LightStats` (`light_policy.rs:211-220`); no gate reads or writes `clusters_enabled` (`rg clusters_enabled crates/boyko_render/src`: writes only at `light_policy.rs:220`, read at `light.rs:1434` inside `collect_lights`' pack) | Allow: the cull joins `LightHeaderWriters` |
| C | `collect_lights` × {ssao, csm, punctual} gates: 3 | R/W | Image | Edge: `.before_set(LightCollectSet)` |
| D | `resolve_shadow_atlas` × 4 gates: 4 | `LightTableDirty` | Benign | Join `LightHeaderWriters` |
| D′ | atlas × punctual gate: 1 | `ResolvedShadowAtlas` W/R | Image | Edge: atlas → gate |
| E | `light_reconcile` × {atlas, cull, cascades}: 3 | light components W/R | Image/sim | Edge: reconcile set before the three |
| F | `resolve_active_camera` × {atlas, cascades}: 2 | `ViewUniform` | Image | `.after_set(CameraSet::Resolve)` |
| G | csm gate × `resolve_csm_cascades`: 1 | `ResolvedCsm` | Image | Edge: `CsmResolveSet` → gate |
| H | particle tick × pack: 1 | `ParticleClock` | Image | Edge: `ParticleTickSet` → pack |
| I | fixture drivers: 3 | `Transform` | Fixture only, not shipped | The fixture orders them |

## Commit sequence

Each commit is green at `cargo test --workspace --all-targets --no-fail-fast` plus `cargo clippy --workspace --all-targets -- -D warnings`.

| # | Commit | Files | Gate (red-first / named mutation) | What moves |
|---|---|---|---|---|
| **K1a** | Ordered apply window + frame obligation | `schedule/schedule.rs:12-33` (doc), `:141-169` (obligation text), `:881-940`, `:1689-1804` (guard doc); `executor_scratch.rs:424-429, 542, 579, 612, 786-794`; new `tests/k1_apply_order_w_sweep.rs` | (1) Unit `apply_window_applies_in_index_order_not_pop_order`: push [3,1,2] into the channel; expect log [1,2,3]. **Red on the parent.** (2) The sweep below. (3) Mutation: restore pop-order apply → (1) and (2) red. EM2′-K unchanged. UG-08 Miri on the schedule unit tests. | Apply order; UG-15 leg (2). Golden moves are not expected; any pin that moves was timing-dependent → re-measure 3× before any bless. |
| **K1b** | Writer-keyed lanes | `event_writer.rs:29-63, 110-166, 194-233`; `events/event_buffer.rs:230-245, 287-333, 340-346, 414-418` (claim/grow); `events/event_dispatcher.rs:134-185, 249-327, 338, 535-559, 631-637`; `ecs_master/event_api.rs:50-52`; `commands/send_event_command.rs:4-61`; `params/commands.rs:344-372` (doc); `schedule_builder.rs:368-425` (scope guard); `system/access.rs:113-123` (doc); `boyko_threadpool/src/tls.rs:396-404` (doc); caller updates: `tests/{event_send_from_worker, phase12_events_systemparam, ke16_app8_consumer_predicates, event_proptest, event_multi_type, event_double_buffer}.rs`, `benches/event_dispatch.rs`, `aether_tests/tests/{a1_bundle_event, a3_machine, a4_machine_hierarchy}.rs` (doc lines 7/13/33) | (1) Two writers on one thread, B sends first → flattened A-block then B-block. **Red today.** (2) D-E20 refusals: 0 at every W. (3) New `event_lanes_survive_growth_and_keep_the_dispatcher_lane_last`: preregister with R = 1; a one-shot (`run_system` + `EventWriter`) sends; an observer runner calls `world.send_event` and a `run_system` writer; build schedule A (writer W1 → lane 1, growth) and schedule B (W2 → lane 2, growth); run one frame; `update_events`. Assert: W1 block, then W2 block, then lane-0 events in dispatcher program order, including those sent before each growth; the one-shot state's lane stays 0. (4) trybuild compile-fail `send_event_needs_exclusive_dispatcher.rs` (`send_event` through `&EcsMaster`). (5) D2e: a `run_cached_system`-initialized writer then added to a builder with W = 4 → `#[should_panic]` (release-active). Mutations: TLS routing → (1) red; flatten 0..L → (3) red; drop the scope guard → (3) red (writers get lane 0 and interleave). | Event order, refusals, memory (D2d) |
| **K2a** | Enable read routes | `filter_enable.rs:181-205, 357-362`; `data_is_enabled.rs:34-36, 127-132, 170-176`; new `iters/query/filter_dyn_enable.rs`; `filter.rs:75-197` (+2 items), `:1547+` (tuple macro); `query.rs:183-218`; `filtered_access_set.rs:121-135, 317-319` (+ `mark_enable_read_all`, `OrderAccum`, `finalize` → `OrderMeta`); `system_meta.rs` (field + accessors); `tests/enable_tag_step9.rs`, `tests/enable_chunk_byte_identity.rs` (+`DynEnableAny`, 13 sites); new trybuild fixture; new root `tests/enable_term_census.rs` | (1) For each route in {`Enabled<R>`, `Disabled<R>`, `IsEnabled<R>`, `Or<(Enabled<R>, With<X>)>`, `DynEnableIn<(R,)>`} assert `meta.access().component_reads ∋ R`. **Red today for all five.** `DynEnableAny` → `READS_ALL_ENABLE` set. (2) Compile-fail: `Query<&P>::with_enabled`. (3) `DynEnableIn<(A,)>::with_enabled(B)` → `#[should_panic]`. (4) Census red on a planted extra site (tester mutation). (5) `#[cfg(debug_assertions)] #[should_panic]` on the finalize premise (O2). Mutation: drop the read in `IsEnabled::init_access` → (1) red. | Access bits; no dispatch change (D5) |
| **K2b** | Declared views | `filtered_access_set.rs:154-274, 301-304` (declared OR under universal); `nonsend_res.rs:98-110`, `nonsend_resmut.rs:75`; `system_meta.rs` (`declare_view`, `ViewDecl`); `dispatcher_token.rs:66-104, 125-181, 252-360` (debug view checks); `schedule.rs:1418-1419` (pass the copy); new `schedule/order_conflict.rs` (Unknown + Data classes) | (1) `NonSendRes` + `Query<&C>` keeps a declared read of C; mutation: drop the OR under universal → red. (2) `GpuCompute` with empty `Access` and no declaration × a reader → `Unknown(GpuUndeclared)`; with `declare_view(read C)` × a writer of C → `Data[C]`. **Red today** (no class exists). (3) `#[cfg(debug_assertions)] #[should_panic]`: a declared `GpuCompute` reads an undeclared resource through `WorldView`. Mutation: remove the view check → (3) red. | `OrderMeta` for about 7 systems per host |
| **K2c** | Deferred declarations | new `params/enable_commands.rs`, `commands/enable_by_id_command.rs`; `params/commands.rs:405-415`; `tuple_impl.rs:110-123`; `system_meta.rs` (`declare_deferred_write`); `order_conflict.rs` (Deferred, DeferredOpen) | (1) `diagnosis_pair_is_a_deferred_conflict`: V = `Commands` + custom toggle of R; each static reader route G_r and `DynEnableAny` → `DeferredOpen`; V × X (G + `NonSendRes`) → `DeferredOpen[R]`; V′ = `EnableCommands<R>` × G_r → `Deferred[R]`; V″ = `EnableCommands<Other>` × G_r → none. **Red before K2** (V×G is none). (2) W1a: `(EnableCommands<A>, UndeclaredProbe)`, where the probe has `HAS_DEFERRED = true` and no declaration, × `Enabled<B>` reader → `DeferredOpen[B]`. Mutation: remove the per-param check → classified closed on {A} → red. (3) O4: `(EnableCommands<R>, Commands)` that spawns then sets → debug assert fires (`#[cfg(debug_assertions)] #[should_panic]`); reversed order → R set. (4) Mutations: remove the open rule → red; remove `EnableCommands`' declaration → V′ none → red. (5) Physics composition + an unordered `spawn_dynamic` spawner in the same builder → 5 × `DeferredOpen[Simulated]`. | New param; `OrderMeta` for holders |
| **K3a** | Analysis, levels, allows, API | new `schedule/ambiguity.rs`; `schedule_builder.rs:101-162`, after `:654`, `:733-751`, `:1084-1172`; `system_config.rs` (after `:135`); `system_descriptor.rs:39-86`; `schedule.rs` (trailing field + getters); `app/app.rs:125-235, 583-631`; `boyko_log` codes + `docs/diagnostics/<code>.md`; new root `tests/ambiguity_allow_census.rs` | proptest `reach` == BFS; a pair ordered only through a set edge is not reported; set-level allow; empty reason → build error; `Error` → `Err` listing the pairs; `Off` → `None` and a `cfg(test)` counter shows the pass never ran; canary: an unordered writer and reader of R → exactly one `Data` pair; census red on a planted allow. | `Schedule` +8 B trailing |
| **K3b** | App ratchet + hwrt leg | new `boyko_app/tests/schedule_ambiguity_ratchet.rs`; `boyko_app/src/plugins.rs:379-425` (env var); `.github/workflows/ci.yml` (step after `:112`); `CLAUDE.md` (new leg) | Must print `running N tests`, N > 0, in both legs. Red controls: (a) delete `.after(propagate)` at `camera_plugin.rs:73` → new pair → red; (b) in-file canary composition adds a `ViewUniform` writer → the diff is exactly that pair; (c) edit one `FROZEN` line without the digest → red; (d) an alias whose `old` is still live → red. First run must reproduce 41/37 `DeferredOpen` (K2 exposure) or the commit explains each difference. | Test and CI only |
| **K3c** | Physics rows | new `boyko_physics/tests/schedule_ambiguity_ratchet_physics.rs` | Same controls; the 7 entry rows in the workspace leg | Test only |
| C1… | Consumers (after K3b) | render/scene plugins, `visibility_sync.rs`, `asset_refcount.rs`, `snap_interpolation.rs` | Each appends to `RESOLVED` or adds a census-counted allow; the ratchet stays green. Golden effects are measured by each consumer commit; re-bless decisions are not this lane's. | D1/D3/D4 → `EnableCommands`; edges C, D′, E, F, G, H; F1 visibility set; validate → gathers; `LightHeaderWriters` allow |

### `k1_apply_order_w_sweep` (D-E0, made stricter)

- **Case `window` (rev 1 shape, O1-corrected):**
  - 8 mutually non-conflicting deferring systems `s0..s7`.
  - Phase-cycled per frame:
    - spawn 4 × (`Tag{sys, ord}`, `Payload`), enabling `Flag` on even `ord`;
    - insert `ExtraA` (even `sys`) or `ExtraB` (odd `sys`) on `ord == 1`;
    - despawn `ord == 3`;
    - disable `Flag` on `ord == 0`;
    - 3 events `(sys, seq)` each.
  - Stress: deterministic busy-wait `((frame·5 + sys·3) mod 8) × 200 µs`.
- **Case `split_retire_guard` (W2):**
  - Registration order Z, N, Q, `D.after(Q)`, `S.after(N)`. N conflicts with Z through `ResMut<Gate>`. Z, N and Q are non-deferring; D and S spawn a `Tag` and send one event.
  - Kahn FIFO gives topological order Z0 N1 Q2 S3 D4 (`schedule_builder.rs:1048-1075`), so idx(S) < idx(D).
  - Full-barrier waves: {Z, Q} → {N, D} → {S}. Expected apply sequence per frame: [D, S].
  - Stress alternates N-slow and D-slow by frame.
- **Runs:** W ∈ {1, 2, 4, 8, 16} × 48 frames, plus 3 runs at W = 8 and 16. Under 2 s.
- **Expected values are analytic:**
  - Per-archetype row order by `Tag` from an in-test swap-remove model (O1). Spawns push; despawn and migration `swap_remove` from the source table and push to the target, applied in ascending (sys, queue order).
  - Hook log ascending.
  - `ArchetypeId` creation order: ExtraA before ExtraB.
  - Enable bits in row order.
  - Events: s0…s7 in 3-blocks, lane-0 events last.
  - `next_fresh`, free-stack size and live count equal across all runs (D3 corollary).
  - `split_retire_guard` sequence [D, S] in every frame.
- **Not asserted:** id ↔ `Tag` identity, or the live id set (D3).
- **Anti-vacuity:**
  - Each system records an end ticket. At W ≥ 2, fewer than 2 distinct end orders fails as "stress did not perturb". The same holds for the (N, D) end order in `split_retire_guard`.
  - A ~30-line reference model with a `split` switch, implementing the retire described at `schedule.rs:147-153`, is run over the recorded completion traces. `split = false` must reproduce [D, S] in every frame, agreeing with the executor. `split = true` must yield at least 2 distinct sequences, which proves the case would go red under a split.

## Costs

| Item | Release frame cost | Build / init | Memory |
|---|---|---|---|
| K1a | Per window: +k bit sets, ⌈n/64⌉ take/zero, k `tzcnt`. About 25 windows per frame at n = 36 gives an estimated 0.05–0.15 µs/frame. n = 1024 means 16 words per window. Receipt: MQ-18 (`ke17_apply_window`, `phase9_scheduler`, before/after, quiet box). | 0 | 0 (reuses the dead field) |
| K1b | −1 TLS read per send; +1 predicted compare per writer run; flatten over L lanes instead of 17 on this box. Receipt: MQ-21. | Growth realloc per claimed lane at build | max(R, writers+1) × cap × size_of E (lanes) plus the same for `reader_buf` |
| K2 | 0 per row; `DynEnableIn` ≤ 8 compares per term push; debug-only 224 B copy per dispatcher-solo dispatch | +~320 B transient per init; `OrderMeta` ≈ 320 B × ~10 = ~3.2 KB | `SystemMeta` stays 256 B |
| K3 | `Off`: one predicted branch in `try_build`; `Schedule` +8 B trailing | `Collect` at n = 36: ~630 pairs, about 10–20 µs; n = 1024: 5–15 ms and 128 KB `reach`; ratchet test < 1 s (4 app builds + 7 physics builds, no device) | report: pairs × (≈ 56 B + items) |

## What this cannot claim

- **K1:**
  - Entity id values or the live id set after a despawn (D3).
  - The Fixed step count (pacing, H-11).
  - `par_iter` reductions (KC-37 (f)).
  - Hook order within one structural op (D-E21).
  - Cross-machine determinism is argued for one binary, not measured.
  - Event order is registration order, not causal order.
  - The KE17 split must keep the frame obligation; this lane only gates it.
  - **`boyko_ui` flow and paint order sorts by `Entity` id** (`boyko_ui/src/layout.rs:537-545`, `:567-570`; `boyko_ui/src/interaction/focus.rs:212-214`). After D1, `Children` order is deterministic, but ids are not (D3), so UI order can still differ across runs (O5).
- **K2:**
  - Untyped structural deferred effects on archetypal components (ruling 4).
  - `Without<T>` absence reads.
  - What a world-exclusive system does.
  - GPU column hazards (`gpu_barrier_inputs`).
  - Hand-written `declare_deferred_write` and `declare_view` are trusted. The view is checked only on paths exercised in debug.
  - A hand-written composite `SystemParam` is one parameter to D7.
  - `set_by_id` targets are outside the O4 assert.
  - **Interior mutability through `Res<T>`:**
    - `ScratchColumn::solve_base` / `solve_view` hand out write-capable pointers through `&self` (`component/scratch/scratch_column.rs:166,180`), and no access analysis sees writes made that way.
    - No instance exists on the simulation path. The 7 `Res<SolverScratch>` consumers (`boyko_physics/src/systems.rs:298,376,589,1039,1174`, `soft/solver.rs:125`, `broadphase_policy.rs:174`) call only `bodies()`, `rows`, `touched` and `bodies_len` readers.
    - The `solve_base` write sites (`resources.rs:1813,1879`) are inside `BroadphaseGrid::emit_passes(&mut self)`.
- **K3:**
  - Filter-disjoint false positives (Bevy #11796).
  - Only engine compositions are pinned.
  - Reason strings are trusted text.
  - A digest change in a non-detector commit is a process violation; the test cannot tell which kind of commit it is in.
  - Closures that share a `type_name` alias ambiguously.
  - An ambiguity here is a deterministic order that an unrelated edit can flip. It is not evidence that any order is correct.

## Validation

- **`debug_assert!`:**
  - `apply_window ⊆ running` and `count_ones == target` after phase 1; the window is zero after phase 2.
  - `reach[i]` has no `j ≤ i`.
  - No non-universal system or declared view writes a bitset id.
  - `SystemMeta` = 256 (const).
  - `claimed_lanes ≤ lanes.len()`.
  - `send_one` lane < lane count.
  - `WorldView` accesses stay inside the declared view (D6b).
  - O4 reserved-not-spawned (D8b).
- **Release-active:**
  - `DynEnableIn` membership.
  - D2e lane-0 guard.
  - `MAX_EVENT_THREADS` growth ceiling.
  - Empty allow reason (build error).
- **Property tests:**
  - `reach` == BFS.
  - The predicate is symmetric.
  - Lane growth preserves every pending and readable event, over random interleavings of send, claim and swap.
- **Benches:** MQ-18, MQ-21, and `schedule_build_ambiguity` (n = 36 host-shaped, n = 1024 synthetic). All record-only.
- **Miri (UG-08):** schedule unit tests; K1b event-buffer growth and flatten.
- **Loom:** N/A. There are no new atomics; growth and flatten run under `&mut`.

## Answers to the critic's open questions

1. **`select_lighting_cull` and `_armed`:** it writes no `_armed` field. It writes `clusters_enabled` in Auto mode and `LightStats` (`light_policy.rs:211-220`). Allow, as group B.
2. **Interior mutability through `Res<T>`:** no instance on the simulation path, and named in the limits. Sites are listed there.
3. **How `EventWriter::init_state` knows it is in a builder:** by the `WriterLaneScope` flag on `EventDispatcher`, set around `schedule_builder.rs:368-425` (D2b).
4. **Initial lane count at `preregister_event`:** `cfg.thread_count` lanes, where lane 0 is the dispatcher lane and the rest are reserved for writers. The engine-default path uses 1. Memory is `max(R, writers + 1) × cap`, with no W term unless the caller passes W (D2d).

## Open questions

1. **Naming:** `DynEnableAny` / `DynEnableIn`. This is a values call, not a performance call. If the owner prefers other names, it is a rename before K2a lands.
2. **Physics rows use the default solver per entry.** If a game-facing entry with a non-default `S` should be pinned too, that is one more table row. No design change.

**Checklist, N/A items:**
- False-sharing padding: no new cross-thread state. `EventBuffer` lanes keep their 64 B pairs.
- Loom: no new atomics.
- Drop order: `OrderMeta` and `ScheduleAnalysis` are owned boxes; growth moves lane pairs without dropping pending events; `ApplyDrainGuard`'s lifetime is unchanged.
- Generation checks: `set_by_id` resolves the live `Entity` at apply; D8b relies on generations only increasing.

**Research used (web, 2026-09-19):**
- [Bevy `MessageWriter`](https://docs.rs/bevy/latest/bevy/prelude/struct.MessageWriter.html) holds `ResMut<Messages<T>>`, so writers of one type never run in parallel.
- [Bevy `QueryState`](https://docs.rs/bevy/latest/bevy/ecs/query/struct.QueryState.html) / [`Query`](https://docs.rs/bevy/latest/bevy/ecs/prelude/struct.Query.html): `transmute_lens` panics when the new terms need access that was not declared. This is the precedent for `DynEnableIn`.
- [Bevy #25797](https://github.com/bevyengine/bevy/issues/25797): dynamic query builders declare their structure up front.
- [Bevy PR #25731](https://github.com/bevyengine/bevy/pull/25731): conflict messages for message readers and writers.
- `order_research.md` (Bevy, flecs, Unity, EnTT) is the base research for rev 1 and stays in force.
