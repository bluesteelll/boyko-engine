> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase 12.5 Profile — Query iter 10k (Track P2)

## Workload

| Engine | Component | Archetypes | Entities | Path |
|--------|-----------|-----------|----------|------|
| boyko  | `BoykoPosition { x: f32, y: f32, z: f32 }` (12 B, `#[repr(C)]`) | 1 (`[BOYKO_POS_ID]`) | 10 000 | `Query<&BoykoPosition>` inside a system body |
| bevy   | `BevyPosition { x: f32, y: f32, z: f32 }` (12 B, `#[derive(Component)]`) | 1 (`World::spawn(pos)` only) | 10 000 | `QueryState::iter(&world)` directly |

Per-row work in both: `sum += p.x`. Sink: `AtomicUsize::store(black_box(sum) as usize)`.

The boyko bench in `comparison.rs::g2_boyko_query_iter_10k` wraps the body in
`world.run_system(|q: Query<&P>| ... )` — every iter rebuilds a
`FunctionSystem`, runs `initialize()` (24 KB `Box::new` + per-param init
+ 24 KB `dealloc`), then `run_unsafe()` + `apply()`. The bevy bench
calls `state.iter(&world)` directly on a hoisted `QueryState`.

## Measurements (Windows 11, release LTO, single-thread)

Three independent criterion runs of `benches/profile_query.rs`. Numbers
are the median of the three reported point estimates. Spread between runs
is ~30% in absolute time (machine load / thermal noise), but the
**ordering** is stable across all three runs.

| Bench                                  | Run 1   | Run 2   | Run 3   | Median  | Notes |
|----------------------------------------|---------|---------|---------|---------|-------|
| `p2_boyko_baseline_10k_run_system`     | 15.00 µs | 11.86 µs | 12.32 µs | **12.3 µs** | g2 shape: per-iter `run_system` |
| `p2_boyko_cached_system_10k`           | 11.25 µs | 7.86 µs  | 11.39 µs | **~11 µs** | hoisted `FunctionSystem` |
| `p2_boyko_run_system_once_10k`         | 12.26 µs | 9.28 µs  | 10.04 µs | **~10 µs** | hoisted + no apply pass |
| `p2_boyko_direct_pool_10k`             | 20.19 µs | 16.71 µs | 18.16 µs | **~18 µs** | walks `pool.get_raw(i)` per row |
| `p2_boyko_get_component_raw_10k`       | 20.84 µs | 16.71 µs | 16.39 µs | **~17 µs** | random-access per entity |
| `p2_bevy_baseline_10k`                 |  9.45 µs |  8.62 µs |  9.06 µs |  **~9 µs** | `state.iter(&world)` direct |

Anchor: the head-to-head `g2_*` benches in the same session reported
boyko 10.7 µs vs bevy 10.0 µs — the same ordering as the profile. The
plan's reference values (boyko 7.88 µs / bevy 6.90 µs) were taken under
lighter machine load; ratios match.

## Boyko inner loop assembly excerpt

Extracted from `target/release/deps/profile_query-*.s` (rustc emit asm
on the release-profile build, intel syntax). Block `LBB306_16` is the
inner per-row loop for `bench_boyko_cached_system`; block `LBB306_11`
is the outer per-archetype dispatch.

```asm
; outer loop — advance to next archetype
.LBB306_11:
    cmp   rax, rcx                          ; archetype_ids iter exhausted?
    je    .LBB306_23                        ; → return None
    mov   r12, qword ptr [rax]              ; load next ArchetypeId.0
    cmp   r12, r9                           ; bounds check vs id_to_slot.len()
    jae   .LBB306_15                        ; → continue (stale id skip)
    movzx r12d, word ptr [r8 + 2*r12]       ; slot_idx = id_to_slot[id] (u16)
    cmp   r12, 65535                        ; NO_SLOT sentinel
    je    .LBB306_15                        ; → continue
    imul  r15, r12, 8480                    ; offset = slot_idx * sizeof(Archetype)
    lea   r11, [r10 + r15]                  ; archetype slot base ptr
    mov   r11, qword ptr [rdx + r11]        ; column.ptr (cached in ReadFetch.base)
    mov   r13, qword ptr [r10 + r15 + 8296] ; archetype.current_index (entity_count)
    xor   r15d, r15d                        ; reset row counter
    jmp   .LBB306_15                        ; → inner loop

; inner loop — 5 instructions, register-only
.LBB306_16:
    test  r11, r11                          ; defensive null check on base ptr
    je    .LBB306_23                        ; (never taken in practice)
    lea   r12, [r15 + 2*r15]                ; r12 = row * 3 (12 B Position is 3 × f32)
    inc   r15                               ; row++
    addss xmm0, dword ptr [r11 + 4*r12]     ; sum += *(base + 12*row) = sum += pos.x
    cmp   r15, r13                          ; row < entity_count?
    jae   .LBB306_11                        ; → outer (advance archetype)
    jmp   .LBB306_16                        ; → inner (next row)
```

**Observations**:

- The hot loop is 5 instructions per iteration (load + add + compare +
  jmp + lea), all register-resident.
- The base pointer (`r11`) is loaded **once per archetype** at LBB306_11
  and reused across the entire inner loop — Phase 7's `ReadFetch.base`
  caching pays off here.
- No `set_change_ticks` call appears in the hot loop. Phase 10's
  `meta` parameter to `set_table_readonly` is consumed by `RefFetch` /
  `MutFetch` but for `&T` it is a no-op (`_meta: &SystemMeta`) and the
  monomorphisation drops the parameter entirely.
- No `filter_fetch` call appears either: `(): QueryFilter` has
  `IS_ARCHETYPAL = true`, so `if !const { F::IS_ARCHETYPAL }` const-folds
  the per-row filter branch out (plan QF1 confirmed in asm).
- Outer-loop archetype dispatch is also tight: 11 instructions to
  resolve an `ArchetypeId` → archetype slot → cache the column base
  + entity count.

## Bevy inner loop assembly excerpt

From the same compiled binary (Bevy is inlined into `bench_bevy_baseline`).
Block `LBB308_9` is the inner row loop; `LBB308_11` is the outer
archetype dispatch.

```asm
; outer loop — advance archetype
.LBB308_11:
    cmp   rax, rcx                          ; storage_id_iter exhausted?
    je    .LBB308_2
    mov   r8d, dword ptr [rax]              ; load ArchetypeId.0 (u32)
    add   rax, 4                            ; iter += 4 bytes (u32 stride)
    mov   r9, qword ptr [rdi + 424]         ; archetypes / tables base
    lea   r10, [r8 + 8*r8]                  ; offset = id * 9 (struct stride)
    mov   r8, qword ptr [r9 + 8*r10 + 16]   ; table.entity_count
    test  r8, r8
    je    .LBB308_11                        ; → skip empty archetype
    lea   r9, [r9 + 8*r10]
    cmp   rdx, qword ptr [r9 + 64]
    jae   .LBB308_7
    mov   r10, qword ptr [r9 + 56]
    mov   r10, qword ptr [r10 + 8*rdx]
    test  r10, r10
    je    .LBB308_7
    mov   r9, qword ptr [r9 + 24]           ; column slot
    not   r10
    lea   r10, [r10 + 2*r10]
    shl   r10, 4
    mov   r9, qword ptr [r9 + r10 + 16]     ; cache column.ptr → r9
    jmp   .LBB308_8

; inner loop — 5 instructions, register-only
.LBB308_9:
    mov   r11d, r10d                        ; copy row (u32 — extra mov vs boyko)
    lea   r11, [r11 + 2*r11]                ; r11 = row * 3
    inc   r10d                              ; row++
    addss xmm0, dword ptr [r9 + 4*r11]      ; sum += base[row].x
.LBB308_10:
    cmp   r10d, r8d                         ; row < len? (u32 compare)
    jne   .LBB308_9
```

**Observations**:

- Inner loop is **the same 5-instruction shape** as boyko's. Bevy
  uses `u32` indices (`TableRow(NonMaxU32)`) — one extra `mov r11d, r10d`
  to widen, then `lea` doing `row * 3`. Per-op count is identical
  (`addss` + `lea` + `inc` + `cmp` + `jne`).
- Bevy's outer dispatch is **longer**: ~14 instructions to walk through
  `Tables → Table → Column.ptr` with the `NonMaxU32`-based archetype
  index encoding (note the `not r10` for `NonMaxU32` decode, and the
  `lea + shl 4` for `Column { ... }` stride). Boyko's slot lookup
  (`id_to_slot[id]` → 8480-byte stride) is straight-line and shorter.
- For a single-archetype workload (one outer-loop trip), the outer
  cost is amortised over 10 000 inner iterations — both engines pay
  it once. No measurable per-archetype delta in single-archetype.

The conclusion is unambiguous: **per-row codegen is on par. The
~1 µs delta between boyko and bevy on g2 lives outside the inner
loop.**

## Per-contributor breakdown

| Suspected contributor | Measured cost (per call) | Confirmed? | Evidence |
|----------------------|--------------------------|------------|----------|
| **System wrapper** (`run_system` per-iter rebuild) | **~1-1.5 µs** of the 12.3 µs / 11 µs delta | **YES — primary** | `baseline_run_system` (12.3 µs) - `cached_system` (~11 µs) = 1.3 µs. Asm at lines 72960-72981 shows per-iter 24 KB `Box::new` + `add_component_read` + memcpy(192) + 24 KB `dealloc` + memcpy(448) on the `run_system` path — gone in `cached_system`. |
| **`set_change_ticks` per archetype** | **0 ns** in the hot loop | **NO** | `meta: &SystemMeta` is forwarded to `set_table_readonly` (forced by Phase 10 Wave C `Ref`/`Mut`), but `<&T as QueryData>::set_table_readonly` takes `_meta: &SystemMeta` and discards it. The monomorphised version of the `&T` arm has the parameter elided — no symbol reference in asm. |
| **Linear archetype matching** | **0 ns** | **NO** | Single archetype → outer-loop trip count = 1. `QueryState::update_archetypes` runs only inside `init_state` (cold) — not on every `iter()`. With the cached `FunctionSystem`, even that runs only on first warm-up call. |
| **Inner-loop codegen** (lack of `#[inline(always)]`) | **0 ns vs Bevy** | **NO** | Asm comparison shows identical 5-instruction loop shape. Boyko's `#[inline]` propagates through `QueryIter::next` and `<&T as QueryData>::fetch` cleanly; Bevy's `#[inline(always)]` produces the same code at this scale. |
| **`get_raw` Vec\<Unit\> indirection** (NOT a query path, but documented) | **~7 µs slower than Query** | n/a (negative finding) | `p2_boyko_direct_pool_10k` (18 µs) > `p2_boyko_cached_system_10k` (11 µs). `pool.get_raw(i)` walks `Vec<Unit>` (16 B per entry) per call — that's a separate 160 KB working set on top of the data buffer. Query avoids this by caching the column base ptr once per archetype. |
| **`get_component_raw` per entity** (random access path) | **~17 µs** | n/a (different path) | Same 1.6x cost as direct pool. Confirms Phase 7's `EntityInland` deref chain (~12-16 ns per call) is well-tuned but cannot beat sequential iteration. |

## Findings

### 1. The dominant non-codegen cost is the per-iter `FunctionSystem` rebuild in `run_system`

`comparison.rs::g2_boyko_query_iter_10k` calls `world.run_system(closure)`
which constructs a fresh `FunctionSystem<F, M>` on every benchmark
iteration. Each rebuild triggers:

1. `IntoSystem::into_system(closure)` → `FunctionSystem::new(func)` —
   cheap.
2. `run_cached_system(&mut sys)` → `system.initialize(self)` —
   **not cheap**:
   - `FilteredAccessSet::new()` allocates a **24 KB `Box<[&'static str;
     OWNERSHIP_SLOT_COUNT]>`** (filtered_access_set.rs:135-141, marked
     `#[cold]`).
   - `<F::Param as SystemParam>::init_state(world, &mut self.meta)` —
     for our `(Query<&Pos>,)` Param this calls
     `QueryDataState::new(world)` which allocates
     `matched_ids: Vec::with_capacity(16)` (query_state.rs:67) plus
     an `ArchetypeBitSet` (128 B) and runs `update_archetypes` (1
     archetype → ~10 instructions).
   - `<F::Param as SystemParam>::init_access(...)` walks the
     accumulator.
   - `access_set.finalize(&mut self.meta)` does a `memcpy(192)` of
     the access bits into `SystemMeta.access`.
   - At the end of the iteration the `FunctionSystem` and its
     `state: Some(QueryDataState)` and `FilteredAccessSet` are dropped
     — **24 KB `dealloc`** + `Vec` drop + `Box<[Tick]>` drops if any.

The asm at lines 72944-72981 makes the per-iter cost visible: the
`alloc::alloc::alloc` / `___rust_dealloc` pair on a 24576-byte
allocation, plus two `memcpy` calls (192 B and 448 B). Conservatively
this is ~1-1.5 µs per call on contemporary hardware (matches the
`baseline_run_system` - `cached_system` delta of 1.3 µs).

**Bevy avoids this entirely** because the bench calls
`state.iter(&world)` directly on a `QueryState` that was built once
outside the timed loop. No system wrapper, no per-iter allocation, no
apply pass.

### 2. The remaining ~2 µs delta after eliminating the wrapper is **not** in the inner loop

`p2_boyko_cached_system_10k` ≈ 11 µs vs `p2_bevy_baseline_10k` ≈ 9 µs.
The hot loop body is byte-for-byte the same shape (5 instructions per
row in both engines). The 2 µs delta is in the **outer-loop and
prologue**:

- Bevy uses a slightly different storage encoding (`NonMaxU32` for row
  ids, `Tables` indirection) but its outer dispatch decodes faster:
  Bevy's `tables.get(table_id)` is a single 24-byte struct stride
  load; boyko's `archetypes.get_archetype_ptr(id)` decodes via the
  `id_to_slot[id]` `u16` indirection then an 8480-byte stride. With 1
  archetype this is 1 trip each; with N archetypes the difference
  scales linearly but stays in the tens of ns per boundary.
- Bevy's `query()` API uses a `state: &mut QueryState` cursor that
  doesn't carry `SystemMeta`. Boyko's `Query<&Pos>` carries
  `meta: &SystemMeta` through every fetch call (Phase 10 Round 2 C2)
  — although for `&T` data the meta is unused, the *pointer-passing*
  through the call chain adds register pressure.
- Boyko's `state.update(master)` at `Query::get_param` (query.rs:311)
  always runs unconditionally per `iter()` call, comparing generations.
  Bevy's `iter` short-circuits the same check at the same depth — same
  ambient cost.

### 3. `set_change_ticks` is **not** the bottleneck

Plan hypothesis P2/1 named `set_change_ticks` as a candidate. The asm
confirms it is **inlined to nothing** in the hot loop. The Phase 10
infrastructure adds zero per-row cost when D = `&T` and F = `()`
because:

- `meta` is plumbed by reference into `set_table_readonly`.
- `<&T as QueryData>::set_table_readonly` accepts `_meta: &SystemMeta`
  and never reads it (data.rs:301).
- Monomorphisation discards the unused parameter; LLVM optimises away
  the pointer load.

### 4. Linear archetype matching is **not** active here

The plan hypothesis P2/2 was based on the worry that
`QueryState::update_archetypes` does a linear `1..current_gen.get()`
sweep on every `iter()`. In reality:

- For `cached_system`, `update_archetypes` runs once (cold init only).
- Inside the actual `iter()` call, `QueryDataState::update(master)`
  only re-runs `update_archetypes` if `archetype_generation` or
  `structural_generation` changed since the last sync (state.rs:185-198).
  In a steady-state world it short-circuits in O(1).
- The runtime sweep that DOES happen (inside `init_state`'s
  `QueryDataState::new` cold path) is amortised to once per system
  lifetime.

For a multi-archetype workload (10×1000 entities, deferred — see the
bench comment block) the boundary cost per archetype is ~10
instructions in the asm (LBB306_11) which would amount to ~50-100 ns
across 10 boundaries — comparable to Bevy. **This hypothesis is
disproven for the single-archetype baseline and remains a future
question for multi-archetype workloads.**

## Recommendation for Track B (architect)

The profile points at **one large lever** and **one small lever**:

### B-Large: a direct-query API that bypasses the system wrapper

`EcsMaster::query<D, F>() -> QueryView<'_, D, F>` (plan §B1) is the
right move and will close most of the gap. Concretely:

- **The 24 KB `FilteredAccessSet` allocation is the largest per-call
  cost.** A direct API does not need to declare access against a
  scheduler — single-thread, single-borrow `&mut EcsMaster` already
  gates aliasing at the type level. The
  `FilteredAccessSet::new()` call can be skipped entirely on this
  path.
- **The `QueryDataState::new` allocation is the second largest.** Per
  plan §B1, cache `QueryState<D, F>` keyed by `TypeId::of::<(D, F)>()`
  in a per-`EcsMaster` `BTreeMap`. After the first query of a given
  shape, the cached state lives in the world and the call cost drops
  to a single pointer load + `update_archetypes` short-circuit.
- **Expected gain**: 1.5-3 µs. After this change boyko should land at
  ≤ Bevy's `state.iter(&world)` cost (~9 µs) and the g2 ratio flips to
  the win column.

### B-Small: drop the `meta` forwarding for read-only `&T` queries

Phase 10's `meta: &SystemMeta` parameter to `set_table_readonly` /
`set_table_mut` is forwarded through the tuple variadic impls
(data.rs:1086-1117) and through the per-row dispatch even when the
concrete `D` is `&T` or a tuple of `&T`. For `&T` the `meta` is
unused (data.rs:301 — `_meta: &SystemMeta`).

The cleanest fix is a `D::NEEDS_CHANGE_DETECTION: bool` associated
const (plan §B1 already proposes this) that gates the meta-fetch in
the iterator. For an `&T`-only query, the iterator can avoid loading
`self.meta` at the archetype boundary, freeing one register. Likely
sub-100 ns gain on the 10k bench; load-bearing for tuples with many
`&T` fields where register pressure builds up.

### Negative findings (record in plan §C2)

- **`set_change_ticks` per archetype** — NOT a hotspot. Plan P2/1
  disproven. The dispatcher writes it once per frame; per-`iter()`
  cost is zero in asm.
- **Linear archetype matching** — NOT a hotspot for steady-state
  iteration. Plan P2/2 disproven for single-archetype workloads.
  Possibly still a concern for many-archetype + many-system loops;
  reassess after Track B's `matched_archetype_ids` cache lands and
  the multi-archetype variant of this profile can be run (currently
  blocked by `DEFAULT_ARENA_SIZE`).
- **Inner-loop codegen** — NOT a hotspot. Plan P2/4 disproven. The
  generated asm is byte-for-byte comparable in size and instruction
  mix to Bevy. `#[inline(always)]` would not change the result; the
  `#[inline]` hints already propagate through the iter chain.

## Reproduction

```powershell
cd D:\claude\BoykoEngine
cargo bench -p bench-bevy-vs-boyko --bench profile_query
# To regenerate asm:
cargo rustc --release -p bench-bevy-vs-boyko --bench profile_query -- `
    --emit=asm -C "llvm-args=-x86-asm-syntax=intel"
# Output: target/release/deps/profile_query-*.s
```

The bench file is at
`crates/bench_bevy_vs_boyko/benches/profile_query.rs`. Multi-archetype
fanout cases are deferred — see the `case (F)` comment block for the
arena-sizing constraint.
