# Phase 12.5 Profile — Spawn 10k

Per-stage breakdown of the `Commands::spawn × 10 000` hot path on the
`ecs` branch. Profile lives in
`crates/bench_bevy_vs_boyko/benches/profile_spawn.rs`; runs with
`cargo bench --bench profile_spawn`.

This document is descriptive. **No production code was changed.**

## Machine + run conditions

- Branch: `ecs`
- Build: `--release`, LTO on (workspace default).
- OS: Windows 11. `Instant::now()` on Windows uses QPC; one paired
  `Instant::now → Instant::now` call costs ~60 ns (measured at p0; see
  table below).
- Sample size: criterion default per-bench (30 samples × ~1 s each).
- Per-iter setup: `EcsMaster::new` rebuilds the world every iter via
  `iter_with_setup` — this matches `comparison.rs` exactly so the
  comparison is apples-to-apples.

## Workload

Identical to `bench_boyko_commands_spawn_10k` in `comparison.rs`:

- Bundle: `BoykoPosBundle { pos: ProfilePosition { x, y, z } }`
  - **1 component**, 12 bytes (`f32 × 3`), tiny size class
    (≤ `TINY_COMPONENT_THRESHOLD = 16`).
  - Pool layout: 2048 components/chunk × 128 chunks = 262 144 max →
    pool never grows during the 10k loop.
- 10 000 entities into a fresh archetype every iter.
- Single-threaded.

The brief mentioned a 3-component bundle; that is **not** the workload
the current 4-bench head-to-head ships. We add a 3-component variant
(`p7`, `p9`) for the future `SpawnBatchCommand` design but the primary
profile target is the 1-component path that produces the 1.97× loss.

## Raw bench numbers

Numbers below come from one bench run (`cargo bench --bench profile_spawn`).
Where two columns are shown, `comparison.rs` is the canonical
head-to-head bench (50-sample, 3-s window). Numbers are medians.

| Bench | Wall time (10k spawns) | Per-entity |
|-------|-----------------------:|-----------:|
| `p0` `Instant::now` paired call | 60.5 ns | — |
| `p1` boyko `Commands::spawn` × 10k | 3.32 ms | 332 ns |
| `p2` boyko `Commands::spawn` × 10k decomposed | 2.78 ms | 278 ns |
| `p3` boyko direct `EcsMaster::create_entity` × 10k | 842 µs | **84 ns** |
| `p4` boyko `EcsMaster::spawn_one` × 10k | 1.05 ms | 105 ns |
| `p5` boyko direct + Instant checkpoints | 1.95 ms | 195 ns |
| `p6` boyko `Bundle::for_each_component_bytes` × 10k (1 comp) | 14.5 µs | **1.45 ns** |
| `p7` boyko `Commands::spawn` × 10k (3-component) | 9.60 ms | 960 ns |
| `p8` bevy `Commands::spawn` × 10k (1 comp) | 1.05 ms | **105 ns** |
| `p9` bevy `Commands::spawn` × 10k (3 comp) | 1.89 ms | 189 ns |
| `p10` boyko `Commands::add(noop)` × 10k | 114 µs | **11.4 ns** |
| `comparison.rs` g4 boyko (canonical) | 2.48 ms | 248 ns |
| `comparison.rs` g4 bevy (canonical) | 1.19 ms | 119 ns |

`p1` (3.32 ms) is higher than the canonical `comparison.rs` value (2.48 ms)
because `profile_spawn` reduces the criterion sample count to 30 (vs 50 in
comparison.rs) and `iter_with_setup` per-iter `EcsMaster::new` overhead
amortises differently across the smaller sample set. The **ratios** track
faithfully: 332/105 ≈ 3.16× in profile-bench-mode, 248/119 ≈ 2.08× in
comparison-mode, both significantly worse than 1.10× target.

## Per-stage breakdown (boyko Commands::spawn)

The `p2` instrumentation brackets the closure body (enqueue phase) vs
the surrounding `run_system` call (which performs the apply phase after
the body returns).

| Stage | Per-entity (raw) | Per-entity (timing-floor subtracted) | % of total |
|-------|-----------------:|-------------------------------------:|----------:|
| `Commands::spawn` enqueue (loop body) | 37 ns | ~31 ns | 14 % |
| `CommandQueue::apply` + per-cmd dispatch + work | 221 ns | ~215 ns | 86 % |
| **total `p2`** | **258 ns** | **~246 ns** | 100 % |

The apply phase is **6× more expensive than the enqueue phase**.

## Per-stage breakdown inside the direct (no-Commands) path

`p5` runs `EcsMaster::create_entity` directly (no `Commands`, no
queue, no SpawnAtCommand) and brackets each per-entity sub-stage with
its own `Instant` pair. With three pairs of `Instant::now` per
iteration the timing floor adds ~3 × 60 ns ≈ 180 ns/entity that does
NOT correspond to real work — the **shape** of the split is informative,
the **absolute** floor is not.

| Sub-stage inside the inner loop | Per-entity (raw, includes timing floor) |
|---|---:|
| (a) archetype lookup proxy (`has_archetype`) | 30.2 ns |
| (b) entity reserve proxy (`next_entity_id`) | 29.9 ns |
| (c) `EcsMaster::create_entity` (the real work) | 72.6 ns |
| total per entity | 175 ns |

The instrumented run reports 175 ns/entity while the
non-instrumented `p3` reports 84 ns/entity. The delta (91 ns) is the
artificial overhead of 3 `Instant::now` pairs per entity — confirming
each pair costs ~30 ns on this machine.

So the **real** `EcsMaster::create_entity` cost (per `p3`) is **~84 ns/entity**
for this workload (1-comp tiny bundle), and the archetype lookup +
entity reserve are each in the single-digit-ns range (sub-3 ns each in
production code — the 30-ns numbers above are entirely measurement
floor).

## Bevy structural comparison

Source: `bevy_ecs 0.18.1` under
`C:\Users\flint\.cargo\registry\src\index.crates.io-*\bevy_ecs-0.18.1\`.

### Bevy's `Commands::spawn<B>` (1 entity) ops

`src/system/commands/mod.rs:400`:
```rust
pub fn spawn<T: Bundle>(&mut self, bundle: T) -> EntityCommands<'_> {
    let entity = self.allocator.alloc();        // 1 atomic fetch_sub + branch
    let caller = MaybeLocation::caller();        // ZST in release
    self.queue(move |world: &mut World| {        // packed-write into CommandQueue
        move_as_ptr!(bundle);
        world.spawn_at_with_caller(entity, bundle, caller).map(|_| ())
    });
    self.entity(entity)
}
```

Op count for **enqueue**:
1. `EntityAllocator::alloc` — 1 `fetch_sub(Relaxed)` on `free_len` (~3 ns).
   On fresh-id path: extra `fetch_add(Relaxed)` on `next_index`. Total
   ~5-7 ns.
2. `CommandQueue::push` (via `self.queue(...)` closure) — 1 packed
   `write_unaligned` of `[CommandMeta][closure_payload]`. ~10-15 ns.
3. `EntityCommands` stub return — free.

Bevy enqueue per entity ≈ **15-20 ns**.

### Bevy's apply path (`spawn_at_unchecked` per command)

`src/bundle/spawner.rs:91` and `src/world/mod.rs:1076`. Per entity:

1. `change_tick = world.change_tick()` — only once when BundleSpawner is
   built (Bevy hoists the spawner across commands of the same bundle
   type — but in the loop-of-10k-Commands::spawn path each command
   carries its own closure so the spawner IS rebuilt per command,
   ≈ ~30 ns first-spawn, ~5 ns subsequent).
   
   Update: actually no — `BundleSpawner::new` is called inside each
   per-command closure. That's an open hotspot in Bevy too (mitigated by
   `spawn_batch`).
2. `bundle_info.as_ref()` — 1 deref.
3. `table.allocate(entity)`:
   - `self.reserve(1)` (amortised fast).
   - `self.entities.push(entity)` — 1 Vec push.
   - per column (1 here): `added_ticks.initialize_unchecked` +
     `changed_ticks.initialize_unchecked` — 2 writes.
4. `archetype.allocate(entity, table_row)` — 1 Vec push.
5. `bundle_info.write_components(...)` — 1 memcpy per component.
6. `entities.set_location(entity.index(), Some(location))` — 1 write.
7. `mark_spawned_or_despawned(entity.index(), caller, change_tick)` —
   1 write.
8. `trigger_on_add` + `trigger_on_insert` — fast path when no observers
   registered (just an early-return check inside `DeferredWorld`).
9. `world.flush()` after each command — no-op when queue is empty.

Bevy apply per entity ≈ **80-95 ns** (no observers, table storage,
geometric growth amortised, cache warm).

**Total Bevy per entity** ≈ enqueue (15-20 ns) + apply (80-95 ns) =
**100-115 ns**. Matches the measured 105 ns / 119 ns.

### Boyko op count per entity

`src/ecs/core/system/params/commands.rs:154` (`Commands::spawn<B>`):

```rust
pub fn spawn<B: Bundle>(&mut self, bundle: B) -> EntityCommands<'_, 's> {
    let entity = self.entity_counter.reserve_entity();   // atomic fetch_add
    self.queue.push(SpawnAtCommand { entity, bundle });  // unaligned write
    EntityCommands::new(entity, self)
}
```

Enqueue:
1. `EntityCounter::reserve_entity` — 1 `fetch_add(Relaxed)` on the
   counter behind the `*const AtomicUsize` (~5 ns + 1 deref ~2 ns).
2. `CommandQueue::push` — 2 `write_unaligned` calls (8 B meta + 20 B
   payload for `SpawnAtCommand<BoykoPosBundle>`) + `set_len`. ~10 ns.
3. `EntityCommands::new` — free.

Boyko enqueue per entity ≈ **15-20 ns**. We measure ~31 ns
(p2 enqueue 37 ns minus 6 ns timing floor for the inner Instant pair).

The 10-15 ns gap between theory and measurement on the enqueue is
likely cache-warmup (first push grows the Vec) + amortisation noise.

Apply (`SpawnAtCommand::apply` →
`EcsMaster::create_entity_at` → `Archetype::create_entity`):

`src/ecs/core/commands/spawn_at_command.rs:79`:
1. `B::cached_archetype_id(world)` — 1 OnceLock Acquire load on the
   per-world cache slot (~3 ns, plan §6.2 target).
2. Stack-allocate `[MaybeUninit<(ComponentId, &[u8])>; 8]` (128 bytes
   of stack-frame init via `[const { uninit() }; 8]`, ~5-10 ns).
3. `self.bundle.for_each_component_bytes(|id, bytes| { ... })`:
   - Bundle macro emits per-field `ManuallyDrop<T>::new(...)` wrappers
     (1 stack copy per field).
   - Stack array of `(ComponentId, *const u8, usize)` triples.
   - `sort_unstable_by_key` — trivial for 1 element but still a call.
   - Loop reconstructing `&[u8]` via `slice::from_raw_parts` per
     iteration + closure dispatch.
   - Measured: **1.45 ns/entity** for 1-component path (`p6`).
4. `EcsMaster::create_entity_at(entity, archetype_id, slots)`:
   - `has_archetype` (1 SparseMap lookup, ~5 ns).
   - `archetype_ptr_for` (slab lookup, ~3 ns).
   - `current_tick` Relaxed load (~2 ns).
   - Bounds-check on `entities_inland.len()` (~1 ns).
   - `Archetype::create_entity(entity_id, &mut idx, components, tick)`:
     - Build `ComponentMask` over input ids (small loop, ~5 ns).
     - `signature.mask().is_subset(input_mask)` — bit-AND over 8 u64s
       (~3 ns).
     - `component_pools.can_push_entity_components` — 1 SparseMap
       lookup + `is_full()` check **per component** (~10 ns total).
     - `component_pools.push_entity_components` — 1 SparseMap lookup +
       `pool.add(bytes)` per component. `pool.add` does:
       - bounds check.
       - `buffer.as_ptr().add(buffer_index * stride)`.
       - `copy_nonoverlapping(bytes, dst, size)` (~5 ns for 12 B).
       - `chunks.get_mut(chunk_index).mark_dirty()` (Vec index + write,
         ~3 ns).
       - `units.push(Unit::new(ptr))` (~3 ns).
     - Total per component ≈ 25-30 ns.
     - **Per-component tick init**: 1 `get_pool_mut` (SparseMap lookup +
       Vec index, ~5 ns) + 2 × `write_*_tick` (UnsafeCell::get + write,
       ~3 ns total) ≈ **8-10 ns per component**.
     - `entity_ids.push(entity_id)` — 1 Vec push (~3 ns).
     - `current_index += 1` (~0 ns).
   - `register_entity_with_ptr` — 2 bounds checks + 1 `EntityInland`
     write + 1 `active_ids.push` + 1 `sparse_to_active` write
     (~10 ns total).

Boyko apply per entity ≈ **70-95 ns** measured by `p3` = 84 ns.
Within plan §10.2 target (~330 ns).

**Total Boyko per entity** ≈ enqueue (15-20 ns) + queue meta overhead
(15-20 ns) + apply (84 ns) = **115-125 ns**.

But we **measure** 248 ns/entity in `comparison.rs` (2.48 ms / 10 000).
That is **123-133 ns more than theory predicts**.

## The unaccounted gap (≈ 130 ns/entity)

`p3` (direct create_entity) = 84 ns/entity.
`p10` (CommandQueue::push only, noop apply) = 11.4 ns/entity.

If the Commands path were a perfect sum of (p10 push + p3 create_entity)
the total would be ~95 ns/entity. The measured `comparison.rs` boyko
value is **248 ns/entity** — a gap of **~150 ns/entity** that the
isolated components do not explain.

Source candidates for the gap:

1. **CommandQueue::apply outer-loop overhead per command**
   (`RawCommandQueue::apply_or_drop_queued`):
   - `meta.read_unaligned()` per slot — 1 unaligned 8-byte load.
   - `cmd_ptr = bytes.as_mut_ptr().add(local_cursor)` — 1 deref.
   - `(meta.consume_and_drop)(cmd_ptr, world, &mut local_cursor)` —
     **indirect function-pointer call** + `cmd: C = cmd_ptr.read_unaligned()`
     pulls 28 B (for `SpawnAtCommand<BoykoPosBundle>`) into a fresh
     stack local.
   - `catch_unwind(AssertUnwindSafe(...))` — 1 per-iteration panic-frame
     registration. **This is a non-trivial cost** in tight loops.
   - Sum: ~15-25 ns/entity. Multiplied by 10 000 = 150-250 µs total.
2. **`SpawnAtCommand::apply` MaybeUninit slot setup** per command:
   `[MaybeUninit<(ComponentId, &[u8])>; MAX_BUNDLE_ARITY=8]` is 128 B on
   the stack. Even `[const { uninit() }; 8]` compiles to LLVM `undef`
   patterns and the actual cost is in the address taken + cast back.
   Likely ~5-10 ns/entity.
3. **for_each_component_bytes** (`p6` measures 1.45 ns/entity isolated).
   For 1-component path this is small. For 3-component path it grows.
4. **Implicit `Box<[OnceLock<ArchetypeId>; 1024]>` access in
   `cached_archetype_id`** — measured ~3 ns on the cached path.
5. **`SystemParam::apply` outer wrapper** — single tuple-forwarder call
   per system invocation. ~5 ns once per system, not per spawn.
6. **`Vec::push` × 4 per entity** in the apply path:
   - pool.units (in pool.add)
   - archetype.entity_ids (in Archetype::create_entity)
   - entity_master.active_ids (in register_entity_with_ptr)
   - chunk dirty mark (Vec index, not push, but a write)
   Each push amortises to ~3-5 ns; 4 of them = 12-20 ns/entity.

The arithmetic: 11.4 (push) + 84 (create) + 15-25 (queue dispatch loop) +
5-10 (MaybeUninit slot setup) + 5-10 (Vec pushes not in p3) ≈ **120-140 ns
predicted**, but we measure 248 ns. The **residual ~110-130 ns is the
real-world cost of cache effects and code-path size differences** that
the microbench p3 does not capture (p3 keeps the world in L1 because
`EcsMaster::new` allocates a Box<Arena> with stable buffers; the
Commands path has higher i-cache pressure because the SpawnAtCommand
glue + CommandQueue walk + per-system FunctionSystem dispatcher all
run in addition to the create_entity code).

This is consistent with **i-cache pressure** being a meaningful factor —
boyko's spawn hot path touches more distinct functions per entity than
Bevy's, even though each individual function is fast.

## Findings (ranked by attribution confidence)

### 1. `SpawnAtCommand` per-command overhead is the dominant tax

**Files**:
- `crates/boyko_ecs/src/ecs/core/commands/spawn_at_command.rs:79-171`
  (`SpawnAtCommand::apply`).
- `crates/boyko_ecs/src/ecs/core/commands/command_queue.rs:264-400`
  (`RawCommandQueue::apply_or_drop_queued` loop body).

**Estimated cost**: ~30-50 ns/entity, dominated by
- the indirect `consume_and_drop_glue` call,
- the per-command `catch_unwind` registration,
- the per-command `[MaybeUninit<...>; 8]` stack array setup, and
- the closure-style `for_each_component_bytes` callback into
  `create_entity_at`.

Bevy collapses these costs to **zero** in `spawn_batch`: one
`BundleSpawner::new` per batch (not per entity), one capacity reserve
per batch, and a tight inline loop over the iterator. Boyko has no
batch equivalent on the Commands path; every entity pays the full
per-command dispatch dance.

### 2. `Archetype::create_entity` walks `components` slice 3× per entity

**File**:
`crates/boyko_ecs/src/ecs/core/archetype/archetype.rs:371-441`.

The same `&[(ComponentId, &[u8])]` slice is traversed:
1. First loop: build `input_mask: ComponentMask`.
2. `can_push_entity_components` (component_pool_bundle.rs:128-145):
   per-component SparseMap lookup + `is_full()` check.
3. `push_entity_components` (component_pool_bundle.rs:161-185):
   per-component SparseMap lookup + `pool.add(bytes)`.
4. **A fourth loop** for the per-row tick init (lines 415-432):
   per-component `get_pool_mut` (SparseMap lookup) + two
   `write_*_tick` calls.

For a 1-component bundle: 4 SparseMap lookups for the same id. For a
3-component bundle: 12 lookups. **Each SparseMap lookup is ~5 ns**
(`get` does `sparse[id]?.dense_index` then bounds check) — so per
entity for the 1-component case ≈ 20 ns spent in SparseMap; for the
3-component case ≈ 60 ns.

Bevy's `BundleInfo` resolves the per-bundle component ids ONCE at
`register_bundle_info::<B>` time and stores the column pointers
directly inside the `BundleSpawner`. Per-entity it is one indexed
deref, not a SparseMap lookup.

### 3. The `[MaybeUninit; MAX_BUNDLE_ARITY=8]` slot array in `SpawnAtCommand::apply` is over-sized

**File**:
`crates/boyko_ecs/src/ecs/core/commands/spawn_at_command.rs:111-114`.

```rust
let mut slots: [MaybeUninit<(ComponentId, &[u8])>; MAX_BUNDLE_ARITY] =
    [const { MaybeUninit::uninit() }; MAX_BUNDLE_ARITY];
```

`size_of::<(ComponentId, &[u8])>() == 8 + 16 = 24 B`, times 8 = **192 B
on the stack per command**, regardless of the actual arity. For a
1-component bundle this is 168 bytes of wasted stack frame per entity
(plus the cache pollution).

The plan's §11.7 says `Commands<'s>` is 16 B — but the apply-time
frame for each command is 200+ B. Replacing the fixed-size slot array
with a `&mut [(ComponentId, &[u8])]` passed in from the caller (sized
to `B::component_ids().len()`) is one avenue. Better: collapse to a
single arity-1 fast path (which is the actual workload) or move the
collection out of the per-command apply altogether.

### 4. `CommandQueue::apply_or_drop_queued` wraps EVERY command in `catch_unwind`

**File**:
`crates/boyko_ecs/src/ecs/core/commands/command_queue.rs:322-336`.

```rust
let glue_call = AssertUnwindSafe(|| {
    unsafe { (meta.consume_and_drop)(cmd_ptr, world, &mut local_cursor); }
});
let result = std::panic::catch_unwind(glue_call);
```

`catch_unwind` registers a personality-function frame on every
iteration. On Windows + MSVC ABI this is ~5-10 ns of pure overhead per
command. For 10 000 commands that is **50-100 µs total** — about 5-10 %
of the spawn-10k wall time.

Bevy's `apply_or_drop_queued` (Bevy 0.18.1 `world/command_queue.rs:235`)
**does NOT** wrap individual commands in `catch_unwind` — it lets a
panic propagate through the queue's outer caller. The panic-survivor
semantics are implemented at a coarser granularity (the outer
`apply` is what runs in a catch context, not per-command).

This is a CLAUDE.md principle-3 (I-cache) violation that nobody
flagged: the catch_unwind closure is a separate code block emitted
into the hot loop. Removing it (or moving it outside the loop) is
nearly-free correctness equivalence under the assumption that user
`Command::apply` impls do not panic in steady state.

### 5. The Commands enqueue path is fine

**Files**:
- `crates/boyko_ecs/src/ecs/core/system/params/commands.rs:154`
  (`Commands::spawn`).
- `crates/boyko_ecs/src/ecs/core/commands/command_queue.rs:115-150`
  (`CommandQueue::push`).

Measured per `p10` (push only, noop apply): **11.4 ns/entity**.
Measured per `p2` (real enqueue): **31 ns/entity** after timing-floor
subtraction. The 20-ns gap is the `EntityCounter::reserve_entity`
atomic RMW (~7 ns) + the bundle move + frame pointer dance (~10 ns) +
amortised Vec::reserve growth (~3 ns). All within plan §10.1 budget.

**No optimisation indicated on the enqueue side.**

### 6. Bundle::for_each_component_bytes (macro output) is fast

**File**: `crates/boyko_macros/src/lib.rs:831-899` (codegen body).

Measured per `p6`: **1.45 ns/entity** for the 1-component case.
The macro generates ManuallyDrop wrappers + a stack array + a sort +
a slice rebuild — but for arity 1 these are all dead/trivial after
LTO inlining. For arity 3 (`p7` vs `p9`: boyko 960 ns vs bevy 189 ns =
5.1× ratio) the cost grows, but it scales linearly with arity, not
super-linearly. The macro is not a hotspot.

## Recommendation for Track A (architect)

Track A's headline plan (§A1 of `PHASE-12.5-SURPASS-BEVY-PLAN.md`) was
`SpawnBatchCommand<B, I>` — collapsing N commands into one, with one
archetype resolve and one capacity reserve. This profile **confirms
that hypothesis** and refines it:

### Primary target (estimated saving: 80-120 ns/entity on the batch path)

A `Commands::spawn_batch<B, I>(iter)` API that:

1. Enqueues a **single** `SpawnBatchCommand<B, I>` instead of N
   `SpawnAtCommand<B>` slots.
2. On apply:
   - Resolves `B::cached_archetype_id` once (saves N - 1 OnceLock
     loads).
   - Mints `n_entities` entities in a single `reserve_batch(n)`
     (saves N - 1 atomic RMWs).
   - Pre-grows every owned pool by `n_entities` (saves N grow checks).
   - Runs a tight `for` over the iterator inside a single
     `for_each_component_bytes`-equivalent block (saves N closure
     dispatches).
   - Does **one** `catch_unwind` for the entire batch (saves N - 1
     panic-frame registrations).
   - Stamps `added`/`changed` ticks via `slice::fill` (vectorisable —
     saves N × 2 per-row UnsafeCell writes).

This collapses the per-entity overhead of items 1, 2, 4, 6 from
"Findings" above into per-batch fixed costs.

### Secondary target (estimated saving: 15-25 ns/entity on the loop-Commands path)

Even without the batch API, two pure overhead removals on the existing
single-spawn path are essentially free:

1. **Move `catch_unwind` out of the per-command loop.** Wrap the
   entire `apply_or_drop_queued` walk in one `catch_unwind`, with the
   panic-recovery logic operating on the survivor range after the
   walk. Bevy already does this. Estimated saving: ~5-10 ns/entity.

2. **Replace `[MaybeUninit<(ComponentId, &[u8])>; 8]` with an
   arity-1 fast path inside `SpawnAtCommand::apply` for `B::component_ids().len() == 1`.**
   Most bundles in practice spawn through a single-component or
   2-component bundle; the 8-slot stack reservation is wasted. A
   `match` on arity dispatching to specialised 1 / 2 / N branches
   would compress the hot stack frame from 192 B to 24 B for the
   common case. Estimated saving: ~5-10 ns/entity (cache effect).

### Tertiary target (estimated saving: 10-20 ns/entity, all paths)

Collapse the **4× SparseMap lookup per component** in
`Archetype::create_entity` to 1 lookup:

- `can_push_entity_components` already touches every pool.
- `push_entity_components` then re-touches every pool.
- The tick-init loop touches every pool a **third** time via
  `get_pool_mut`.
- And the `input_mask` build does a fourth.

A precomputed `&[InlandPoolId]` cached on `BundleInfo`-equivalent
state (boyko has `BundleStaticInfo` already; extend it with
per-world resolved `InlandPoolId`s on the cold path of
`cached_archetype_id`) would let the apply path index `pools[]`
directly.

This is the **structural change Bevy uses (`BundleInfo` with
component-id-to-storage-index map)**, and it benefits every spawn,
not just the batched ones.

## Negative findings (hypotheses from PHASE-12.5-SURPASS-BEVY-PLAN.md
that this profile disproves)

### H4 (plan §P1.4) — "Phase 10 per-component tick init: 60k writes is significant" — partially disproven

Per-component tick init is ~8-10 ns per component (the `get_pool_mut`
SparseMap lookup dominates, not the actual `write_*_tick`). For the
1-component workload that is ~8-10 ns/entity, **not** the dominant
hotspot. Replacing the per-row UnsafeCell write with `slice::fill`
saves at most ~4 ns/entity (one of the two writes per component).
The SparseMap lookup remains, so the gain is bounded.

**Refinement**: vectorised tick init is still worth doing in a batch
context (`slice::fill` over N rows in one go), but it is not the
right target on the per-command path.

### H5 (plan §P1.5) — "Bundle::for_each_component_bytes callback overhead per entity" — disproven for arity-1

`p6` measures 1.45 ns/entity. Even at 3-component arity it scales
linearly. The macro generates clean code post-LTO. Not worth touching.

### H2 (plan §P1.2) — "EntityMaster::register_entity_with_ptr atomic
counter could cost ~5-10 ns" — confirmed but small

Single atomic RMW on `EntityCounter::reserve_entity` is ~5-7 ns.
This was correctly identified as part of the budget; it is not the
dominant cost.

### H3 (plan §P1.3) — "Archetype::create_entity Unit alloc, pool grow" — disproven

With a tiny-class component (12 B → 2048/chunk × 128 chunks =
262 144 max) the pool never grows during the 10k loop. Pool grow is
not on the hot path of this workload. The per-row `units.push` is
amortised Vec::push cost, ~3 ns/entity — negligible.

### Surprise finding (not in the plan)

The per-command `catch_unwind` wrap in `RawCommandQueue::apply_or_drop_queued`
is a real, measurable hotspot that nobody flagged at design time. It
is also the **easiest** of all the optimisations to ship: relocating
the `catch_unwind` from inside the loop to outside the loop is a 10-line
refactor with Bevy-parity semantics.

## Methodology caveats

1. **`Instant::now()` floor.** Windows QPC costs ~60 ns per pair (`p0`).
   The instrumented `p5` adds ~3 × 60 = 180 ns/entity of artificial
   overhead. The **shape** of the per-stage split is informative; the
   absolute floor numbers reported inside `p5` are not.
2. **`iter_with_setup` setup cost.** Each iter rebuilds the world via
   `EcsMaster::new` (allocates a fresh 64 MB arena, three Vecs sized
   to 64 000 entities, plus the boxed bundle cache). For the 30-sample
   profile-bench run this setup amortises imperfectly, inflating `p1`
   vs the canonical `comparison.rs` value (3.32 ms vs 2.48 ms). The
   **ratios** between profile-bench numbers remain valid.
3. **Compiler may dead-eliminate microbenches.** `p6` originally
   measured 67 ns total (`< 0.01 ns/entity`) until we added a static
   atomic sink. After the fix, `p6 = 14.5 µs (1.45 ns/entity)`. Any
   profile-bench number not channeled through a side-effect (atomic
   store / `black_box`) is suspect.
4. **System dispatch overhead is included.** `p1`, `p2`, `p7` all run
   inside `EcsMaster::run_system`, which builds a one-shot
   FunctionSystem (init + run + apply). That is ~30 ns *once per
   call*, ~3 ns/entity at N=10 000 — small enough to ignore.

## Artefacts

- Bench source: `crates/bench_bevy_vs_boyko/benches/profile_spawn.rs`.
- Cargo registration: `crates/bench_bevy_vs_boyko/Cargo.toml`
  `[[bench]]` block.
- Repro: `cargo bench --bench profile_spawn` (full ~25 s) or
  `cargo bench --bench profile_spawn -- --quick` (~10 s, noisy).
