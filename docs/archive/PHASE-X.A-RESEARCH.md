> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase X.A Research — Batched / Chunked Query Iteration API

This document is the researcher's input to the architect for designing
`Query::for_each_chunk` (and the parallel `par_for_each_chunk`). It covers
prior art in flecs, Bevy, EnTT, Unity DOTS, plus Rust-specific SIMD
reduction techniques. All numbers and quotes are sourced; see §11.

## TL;DR

1. **Closure-per-chunk is the dominant shape in C-style ECS** (flecs `run`/`each`,
   Unity DOTS `IJobChunk.Execute`). EnTT and Bevy never shipped one; Bevy's
   PR #6161 (the only Rust prior art) was **closed for inactivity in October
   2024** with the `S-Adopt-Me` label, leaving boyko-engine a real
   differentiator opportunity.
2. **Strategy (a) "one slice per archetype, user handles tail"** is what flecs
   ships. Strategy (b) "engine pre-chunks to lane width" is what Bevy PR #6161
   attempted and is what burned its complexity budget (alignment generics,
   padding semantics). Strategy (c) "pad columns" is **not used by any of the
   four engines** — the memory and cache cost dominates.
3. **`fadd_algebraic` (and `f32::algebraic_add`) is still nightly-only as of
   Rust 1.95** (May 2026) under `#![feature(float_algebraic)]`. Tracking issue
   `rust-lang/rust#136469` is in FCP `proposed-final-comment-period` /
   `disposition-merge` but no stabilization PR has landed. `std::simd` is
   likewise still nightly with no near-term timeline. This is a **major
   constraint** on what the bench harness can demonstrate.
4. **Bevy 0.18's iter is already vectorizable for raw `&T` reads** via the
   `Iterator::fold` override (PR #6773, merged 2023-12-01) — the byte-identical
   asm finding in Phase 12.6 is *not* a Bevy weakness but the bench's
   `acc += black_box(p.x)` poisoning the reduction. The bench must change to
   `slice.iter().copied().fold(0.0_f32, f32::algebraic_add)` (or equivalent)
   for the for_each_chunk speedup to be visible.
5. **Per-row `Tick`-based filters (`Added<T>`, `Changed<T>`) structurally
   cannot compose with a contiguous-slice API** — the filter has to read each
   row's tick and either skip it or include it. Same constraint Bevy hit.
   Three viable behaviours: refuse-filter (compile error), yield-with-mask
   (slice + bitmask), or fall-back-to-scalar.

---

## 1. Prior art — exact API shapes

### 1.1 flecs (C and C++) — "user owns the inner loop" is the documented pattern

**C API** ([flecs/docs/Queries.md, master branch](https://github.com/SanderMertens/flecs/blob/master/docs/Queries.md)):

```c
ecs_iter_t it = ecs_query_iter(world, q);
while (ecs_query_next(&it)) {
    Position *p = ecs_field(&it, Position, 0);
    Velocity *v = ecs_field(&it, Velocity, 1);
    for (int i = 0; i < it.count; i ++) {
        p[i].x += v[i].x;
        p[i].y += v[i].y;
    }
}
```

The flecs docs ([Queries.md](https://www.flecs.dev/flecs/md_docs_2Queries.html))
state verbatim: *"Iteration is split up into two loops: the outer loop which
iterates tables, and the inner loop which iterates the entities in that
table … This approach provides direct access to component arrays, which
allows compilers to do performance optimizations like auto-vectorization."*

- **Shape**: raw `T*` pointer + `int32_t count`. Not a closure, not an
  iterator — the user writes the `while`/`for` themselves.
- **`ecs_field` signature** ([flecs Iterators API](https://www.flecs.dev/flecs/group__iterator.html)):
  `#define ecs_field(it, T, index)` wraps
  `void *ecs_field_w_size(const ecs_iter_t *it, size_t size, int8_t index)`.
  Indices map to query term order, **starting at 0**.
- **Tail handling**: NONE at API level — `it.count` is the actual per-table
  entity count. The user's inner loop handles whatever shape (4-wide, 8-wide,
  scalar tail).
- **Multi-component shape**: parallel `T*` arrays, all with length `it.count`.
  They are guaranteed same-length per table because flecs is archetype-based.
- **Filter composition**: for `Not` terms — *"Fields for terms that uses the
  `Not` operator will never provide data"*. For `Optional` terms the user
  MUST call `ecs_field_is_set(&it, term_index)` before accessing the field.
  So filters DO compose with the columnar shape, but optional access is
  explicit and per-table.
- **Documented as user-owned**: yes — the `run` callback variant (see C++
  below) is described as *"leaves iteration up to the callback implementation"*.

**C++ API** — flecs exposes three entry points:

```cpp
// 1. each() — engine owns the inner loop (one entity per call):
q.each([](Position& p, const Velocity& v) {
    p.x += v.x;
    p.y += v.y;
});

// 2. run() — user owns the outer AND inner loop:
q.run([](flecs::iter& it) {
    while (it.next()) {
        auto p = it.field<Position>(0);
        auto v = it.field<Velocity>(1);
        for (auto i : it) {
            p[i].x += v[i].x;
        }
    }
});

// 3. each_iter (Rust binding variant) — provides the flecs::iter alongside.
```

Documentation calls `each` *"the default and often fastest approach"* in
C++, while `run` is the explicit user-driven loop.

**Parallel** ([flecs Iterators API](https://www.flecs.dev/flecs/group__iterator.html)):

```c
ecs_iter_t ecs_worker_iter(const ecs_iter_t *it, int32_t index, int32_t count);
```

Splits an iterator into `count` worker slices; each worker drives its own
`ecs_query_next`/`ecs_worker_next` chain. Distribution is documented as
stable across workers.

**Sander Mertens's design rationale** ([Building an ECS #2: Archetypes and Vectorization](https://ajmmertens.medium.com/building-an-ecs-2-archetypes-and-vectorization-fe21690805f9)):
the SoA-per-archetype layout is **explicitly designed to expose contiguous
slices** so that the user's inner loop is what the compiler auto-vectorizes.
flecs does not insert its own SIMD; it gets out of the compiler's way.

### 1.2 Bevy 0.18 — no chunked API; PR #6161 closed unmerged

Bevy's current `Query::iter()` returns `QueryIter`, an
`Iterator<Item = D::Item>`. There is **no slice-returning entry point**.
The relevant performance optimization that ships in Bevy is the
`Iterator::fold` override on `QueryIter`
(PR [#6773, merged 2023-12-01](https://github.com/bevyengine/bevy/pull/6773)).
The PR notes:

> *"Query::for_each, Query::for_each_mut, Query::for_each, and
> Query::for_each_mut have been moved to QueryIter's Iterator::for_each
> implementation, and still retains their performance improvements over
> normal iteration."*

PR author also acknowledges the limitation: *"Ideally, Query::iter and
friends should be able to achieve the same results. However, this does seem
to be blocked upstream by Rust's loop optimizations."*

The fold override calls three storage-specific helpers (current
`crates/bevy_ecs/src/query/iter.rs`):

```rust
pub(super) unsafe fn fold_over_storage_range<B, Func>(
    &mut self, mut accum: B, func: &mut Func,
    storage: StorageId, range: Option<Range<u32>>,
) -> B where Func: FnMut(B, D::Item<'w, 's>) -> B;

pub(super) unsafe fn fold_over_table_range<B, Func>(
    &mut self, mut accum: B, func: &mut Func,
    table: &'w Table, rows: Range<u32>,
) -> B;

pub(super) unsafe fn fold_over_archetype_range<B, Func>(
    &mut self, mut accum: B, func: &mut Func,
    archetype: &'w Archetype, indices: Range<u32>,
) -> B;

pub(super) unsafe fn fold_over_dense_archetype_range<B, Func>(...)
```

These walk per-archetype/per-table ranges with a tight loop, allowing LLVM
to vectorize the **inner per-row body** — but the closure still receives
`D::Item<'w, 's>` per row, not `&[T]`. The fold is an internal-only API;
users get vectorization "by accident" only when they write
`query.iter().fold(...)` and LLVM can prove the closure trivializes.

**Bevy issue [#1990 "Batched ECS Query"](https://github.com/bevyengine/bevy/issues/1990)**
is the canonical request: status **OPEN** with labels
`A-ECS, C-Feature, C-Performance, D-Complex, S-Adopt-Me`. No assignees. No
milestone. OP from 2021: *"More optimizations could be done in systems that
could iterate over queries in a batched, packed way rather than individually."*

**Bevy PR [#6161 "Implement batched query support"](https://github.com/bevyengine/bevy/pull/6161)**
by InBetweenNames (the only serious attempt): **closed October 6, 2024 for
inactivity**, marked `S-Adopt-Me`. The proposed final API:

```rust
query.for_each_mut_batched::<N>(
    |scalar_item| { /* scalar prologue + tail */ },
    |batched_item| { /* SIMD batch path */ }
)
```

Earlier iterations had explicit alignment generics:

```rust
for_each_mut_batched::<4, Align16>(|((mut a, Align16), (b, Align32))| { ... })
```

The PR introduced a `MAX_SIMD_ALIGNMENT` constant and a `SimdAlignedVec`
wrapper around `BlobVec` to ensure column starts were SIMD-aligned so
batch 0 had no scalar prologue. Reviewer concerns that killed the PR:

- **workingjubilee**: the example used `_mm_loadu_ps` on `Vec3` (12 bytes),
  reading uninitialized lane-3 padding — a soundness hole.
- **BoxyUwU**: suggested waiting for `generic_const_exprs` stabilization to
  do per-component automatic alignment; flagged unsafe pointer-alignment
  checks.
- **alice-i-cecile**: demanded discoverability work — `examples/bevy_ecs`
  examples explaining when/why to use it.
- **james7132**: wanted the feature but cited generic-const-exprs blocker.

The fundamental issue per the author: *automatic per-component alignment
requires `generic_const_exprs`* on stable Rust. Without it, the user has to
specify alignment manually, killing the ergonomics.

**Key inference for boyko**: the path of least resistance is the **flecs
shape** (closure receives a `&[T]` slice or pair of slices; engine does no
alignment generics; user handles their own tail). Bevy tried (b) "engine
pre-chunks to SIMD width" and the alignment-generics machinery sank the PR.

### 1.3 EnTT — no chunked API; documented as deliberate omission

**Issue [#462 "Iteration over continuous intervals of components"](https://github.com/skypjack/entt/issues/462)**
by Raikiri proposed exactly the API the architect is considering:

```cpp
registry.view<position, velocity>().each([dt](auto *pos, auto *vel, size_t count){
  for(size_t offset = 0; offset < count; offset += 4) {
    MulAdd4(pos + offset, vel + offset, dt);
  }
});
```

Status: **CLOSED**. The visible page content doesn't include skypjack's
resolution comment, but EnTT today exposes only `.each(scalar_closure)` and
direct iteration over `view::iterator`. There is no public per-chunk slice
API in the EnTT C++ API as of mid-2026 (last verified via
[EnTT crash course](https://github.com/skypjack/entt/wiki/Crash-Course:-entity-component-system)).

EnTT's `group` (vs `view`) is the closest thing — it guarantees a
perfectly-packed contiguous storage for the owned components and lets you
call `group.raw<T>()` to get a `T*`. But you have to do the iteration
outside the registry, which is the manual form of "engine yields a slice".

### 1.4 Unity DOTS — `IJobChunk.Execute(in ArchetypeChunk, ..., bool useEnabledMask, in v128 chunkEnabledMask)`

[Unity DOTS Entities 1.0 docs](https://docs.unity3d.com/Packages/com.unity.entities@1.0/manual/iterating-data-ijobchunk-implement.html):

```csharp
[BurstCompile]
public struct UpdateTranslationFromVelocityJob : IJobChunk
{
    public ComponentTypeHandle<VelocityVector> VelocityTypeHandle;
    public ComponentTypeHandle<ObjectPosition> PositionTypeHandle;
    public float DeltaTime;

    [BurstCompile]
    public void Execute(in ArchetypeChunk chunk, int unfilteredChunkIndex,
                        bool useEnabledMask, in v128 chunkEnabledMask)
    {
        NativeArray<VelocityVector> velocityVectors = chunk.GetNativeArray(ref VelocityTypeHandle);
        NativeArray<ObjectPosition> translations  = chunk.GetNativeArray(ref PositionTypeHandle);

        var enumerator = new ChunkEntityEnumerator(useEnabledMask, chunkEnabledMask, chunk.Count);
        while (enumerator.NextEntityIndex(out var i)) {
            float3 translation = translations[i].Value;
            float3 velocity    = velocityVectors[i].Value;
            translations[i] = new ObjectPosition { Value = translation + velocity * DeltaTime };
        }
    }
}
```

- **Shape**: closure-per-chunk (the `Execute` method is the callback; the
  job system invokes it once per matched chunk).
- **Tail handling**: a chunk in DOTS is a **fixed-size 16 KiB memory block**,
  so the inner loop runs over `chunk.Count` which is whatever fits. The user
  writes `for (int i = 0; i < chunk.Count; i++)`. There's no SIMD-lane-width
  concern at the API surface — Burst (LLVM) vectorizes the inner loop.
- **Multi-component**: `chunk.GetNativeArray(ref Handle)` returns a
  `NativeArray<T>` per type. Same-length-per-chunk is guaranteed because
  all entities in a chunk share an archetype.
- **Filter composition** (enabled components): the `useEnabledMask` +
  `chunkEnabledMask: v128` pair encodes per-row enable bits. The
  `ChunkEntityEnumerator` iterates only enabled rows — i.e., **the user's
  inner loop becomes scalar-per-active-row** when filtering is active. This
  is exactly the trade-off boyko's Phase 10 change detection will face.
- **Parallel**: `IJobChunk.Schedule(query, dependsOn)` and
  `ScheduleParallel(query, dependsOn)` — the job system fans chunks out
  across worker threads. Filter mask is computed per chunk.
- **Documented as user-owned**: yes — the user writes the `while/for`
  inside `Execute`.

### 1.5 Comparative API-shape table

| Engine | Entry point | Callback signature | Multi-component shape | Tail handling | Filter composition |
|---|---|---|---|---|---|
| **flecs C** | `ecs_query_next + ecs_field` | raw `T*` + `it.count`, no callback | parallel arrays, all length `it.count` | user writes inner loop | `Not` → no slice; `Optional` → `is_set` per term |
| **flecs C++ `run`** | `q.run(lambda(iter&))` | `flecs::iter& it`, `it.field<T>(i)` | iter exposes typed `field<T>` per term | user writes inner loop | identical to C |
| **flecs C++ `each`** | `q.each(lambda(T&...))` | per-row component refs | refs, scalar | engine drives loop | filters resolved by engine |
| **Bevy 0.18** | `query.iter().fold(...)` or `query.iter()` | per-row `D::Item` | tuple per row | engine drives loop | `Changed<T>` skips rows individually |
| **Bevy PR #6161 (closed)** | `for_each_mut_batched::<N>(scalar, batched)` | dual closure: scalar + batched-of-N | tuple of aligned arrays | engine emits batch 0 aligned + scalar prologue/tail | n/a (PR did not address) |
| **EnTT view** | `view.each(lambda)` or iter | per-row component refs | refs, scalar | engine drives loop | per-row include/exclude |
| **EnTT group** | `group.raw<T>()` + manual loop | raw `T*` + `group.size()` | one slice per group | user writes inner loop | enforced at group-construction time |
| **Unity DOTS** | `IJobChunk.Execute(chunk, ...)` | `ArchetypeChunk` (16 KiB block) | `chunk.GetNativeArray<T>()` per handle | user writes inner loop over `chunk.Count` | `ChunkEntityEnumerator` skips disabled rows |

---

## 2. Rust-specific SIMD reduction techniques

### 2.1 `fadd_algebraic` / `f32::algebraic_add` — **still nightly as of May 2026**

[std::intrinsics::fadd_algebraic](https://doc.rust-lang.org/std/intrinsics/fn.fadd_algebraic.html) (Rust 1.95):

```rust
#[unstable(feature = "core_intrinsics", ...)]
pub const fn fadd_algebraic<T>(a: T, b: T) -> T where T: Copy;
```

Documentation: *"Float addition that allows optimizations based on
algebraic rules."*

The user-facing variants `f16::algebraic_add`, `f32::algebraic_add`,
`f64::algebraic_add`, `f128::algebraic_add` exist behind
`#![feature(float_algebraic)]`. Tracking issue
[rust-lang/rust#136469](https://github.com/rust-lang/rust/issues/136469) —
status as of May 2026: **in proposed-final-comment-period with
`disposition-merge`, no stabilization PR landed yet**.

Per [The state of SIMD in Rust in 2025 (Shnatsel)](https://shnatsel.medium.com/the-state-of-simd-in-rust-in-2025-32c263e5f53d):
*"There is a way to tell the compiler not to worry about precision loss,
but it's currently nightly-only."*

**Workload + numbers from [orlp.net "Taming Floating-Point Sums"](https://orlp.net/blog/taming-float-sums/)**
(extracted via the Lobsters discussion and an earlier WebFetch on the
article itself):

- **CPU**: AMD Threadripper 2950X.
- **Compiler**: nightly Rust with `RUSTFLAGS=-C target-cpu=native`,
  `--release`. Feature gate: `#![feature(core_intrinsics)]`.
- **Workload**: sum 100,000 random `f32` ∈ [-100,000, +100,000] — 400 KB
  working set.
- **Throughput**:
  - `naive` (sequential `fadd`, no autovec, IEEE strict): **5.5 GB/s**.
  - `naive_autovec` (uses `fadd_algebraic` in the inner loop, LLVM
    autovectorizes): **118.6 GB/s**.
  - `block_kahan_autovec` (chunked Kahan compensation, still autovec via
    `fadd_algebraic`): **98.0 GB/s** with mean-absolute-error 1.23 (vs
    14.54 for `naive_autovec`).
- **Speedup: 21.6× (5.5 → 118.6 GB/s)** for the naive sum when
  `fadd_algebraic` is used to let LLVM reorder partial sums into SIMD lanes.

**Bench harness change required for boyko's Phase X.A demonstration**:

- Change `acc += black_box(p.x)` → `acc = f32::algebraic_add(acc, p.x)`
  inside the closure body OR
  `slice.iter().copied().fold(0.0_f32, f32::algebraic_add)` outside the
  closure. The `black_box` per-element is the autovec inhibitor, not the
  engine.
- Requires `#![feature(float_algebraic)]` on the bench crate — boyko
  already uses nightly for `portable_simd` exploratory work? Verify. If
  staying stable, fall back to manual `std::arch` AVX2 intrinsics in the
  bench (`_mm256_add_ps`).

### 2.2 `std::simd` (portable SIMD) — **still nightly as of May 2026**

[Tracking issue rust-lang/rust#86656](https://github.com/rust-lang/rust/issues/86656)
for RFC 2948: still open. Per Shnatsel's 2025 survey: *"std::simd is
nightly-only and will remain such for the foreseeable future, so it's
unusable in most situations."* Blockers: mask type design
(`Simd<i32, N>` vs dedicated mask), swizzle API, lane count generic story.

Recommendations from the Rust SIMD community (Shnatsel, 2025) for stable
Rust today:

- **`wide`** crate — portable SIMD on stable, no multiversioning.
- **`pulp`** — higher-level, better docs, runs on stable, multi-version-aware.
- **`macerator`** — newer alternative.
- **`std::arch`** intrinsics — stable since Rust 1.27, available for
  x86/x86_64 (AVX2, AVX-512 via `#[cfg(target_feature)]`).

For the bench harness alone (not the engine API), `std::arch::x86_64::*`
is the lowest-friction stable path. AVX2 baseline matches boyko's stated
target platform.

### 2.3 Auto-vectorization triggers — what LLVM reliably vectorizes for `&[f32]`

Nick Wilcox's blog posts ([autovec](https://www.nickwilcox.com/blog/autovec/),
[autovec2](https://www.nickwilcox.com/blog/autovec2/)) demonstrate the
pattern, though they focus on interleave/deinterleave rather than
reductions. Distilled requirements (cross-referenced with the
[Rust Performance Book](https://nnethercote.github.io/perf-book/) and the
orlp.net article):

1. **Slice-typed input, not raw pointers** — bounds-check elision proof.
2. **No iterator state across loop iterations EXCEPT a single reduction
   variable** — and that variable must allow reassociation. Strict-IEEE
   `f32 +` does **NOT** allow reassociation, so the compiler refuses to
   vectorize. `fadd_algebraic` is the unblocker.
3. **Predictable strides** — `for x in slice` or
   `for i in 0..n { use slice[i] }` both work; complex strides like
   `slice[i*3+1]` typically don't.
4. **No `black_box` inside the loop body** — `black_box` is an opaque
   optimization barrier; any element it touches is treated as escaping.
5. **`#[repr(transparent)]` / `#[repr(C)]` for newtype wrappers** —
   guarantees the layout-equivalent stride for the SIMD load.

The reliable shape for an f32 sum reduction on nightly:

```rust
#![feature(float_algebraic)]
slice.iter().copied().fold(0.0_f32, f32::algebraic_add)
```

LLVM emits an unrolled VADDPS loop with N partial accumulators, plus a
horizontal reduction tail. Confirmed by godbolt inspection patterns
documented in the orlp article.

### 2.4 `chunks_exact` vs `chunks` for tail handling

The canonical Rust idiom for explicit-vector-width loops (when not relying
on the autovectorizer):

```rust
let chunks = slice.chunks_exact(W);   // returns ExactChunks: never includes the tail
let remainder = chunks.remainder();    // the 0..W leftover elements
for chunk in chunks {
    // chunk: &[T] with exact length W — compiler proves it
}
for &x in remainder {
    // scalar
}
```

`chunks_exact` is preferred over `chunks` because:

- The yielded slice has a **compile-time-knowable inner length** (the W
  argument), which LLVM uses to unroll.
- `remainder()` is a separate cold path — the hot loop is uniform.
- `chunks(W)` yields slices of varying length, defeating unroll.

Rust Reference: `slice::chunks_exact` since 1.31.0.

### 2.5 Alignment — must boyko's component pool be 16/32/64-byte aligned?

Modern x86 verdict (Intel Optimization Manual §3.6, AMD Software
Optimization Guide §15.1):

- **vmovups (unaligned 256-bit load)**: no penalty vs `vmovaps` (aligned)
  on Haswell+, Zen+ when the load **does NOT cross a 64-byte cache line
  boundary**.
- **Cross-cache-line cost**: 128-bit loads crossing a CL split to ~½ rate
  (1 load per 2 cycles vs 1 per 1). 256-bit loads crossing a CL drop to
  **~¼ rate** (1 load per 4 cycles).
- **AMD Zen**: similar — Zen 3/4/5 have no measured aligned-vs-unaligned
  penalty within a cache line.

**Implication for boyko**:

- `ComponentPool::buffer_ptr()` returns a column start. If column starts
  are 32-byte aligned (AVX2 register width), every aligned 256-bit load
  from start lands within a 64-byte cache line until row count hits the
  unaligned remainder.
- 64-byte alignment is needed only for AVX-512 baselines. Current boyko
  target is AVX2 — **32-byte alignment is sufficient**.
- For non-power-of-2 component sizes (e.g., a `struct Foo([f32; 3])` of
  12 bytes), the column itself is naturally interior-misaligned per row,
  regardless of column-start alignment. **This was exactly Bevy PR #6161's
  `Vec3` soundness blocker.** boyko's API should NOT promise SIMD-aligned
  per-row access for arbitrary T; only the column start.

**Current state in boyko**: `ComponentPool::buffer_ptr() -> *const u8`
exists at `crates/boyko_ecs/src/ecs/memory/component_pool.rs:732`. Whether
that pointer is 32-byte aligned today needs verification by the architect
(or developer during Phase X.A); the Arena layer may or may not impose
alignment.

---

## 3. Tail-handling design space

Strategies as posed in the brief:

### (a) "One slice per archetype, user handles tail"

- **Used by**: flecs (C `it.count` + `ecs_field`), Unity DOTS
  (`chunk.Count`), EnTT groups (`group.raw<T>()` + `group.size()`).
- **Pros**: minimal engine complexity; column data is untouched; the
  user's choice of vector width is local; matches the
  `chunks_exact + remainder` Rust idiom naturally; multi-component case
  trivially gives same-length parallel slices.
- **Cons**: every caller writes tail-handling boilerplate. Mitigatable
  with a tiny `simd::chunked_sum(slice)` helper crate-internal to boyko.

### (b) "Engine pre-chunks to lane width N"

- **Used by**: nobody in production. Bevy PR #6161 attempted it — closed.
- **Pros**: hot loop sees only `[T; N]`-shaped blocks; user code is cleanest.
- **Cons**: forces an alignment generic on the API
  (`for_each_mut_batched::<N, AlignA>`); requires `generic_const_exprs` for
  per-component automatic alignment; padding semantics are fragile (Bevy
  PR #6161 reviewed lane-3 reading uninitialized `Vec3` padding bytes as a
  soundness hole); double-callback shape
  (`scalar_prologue + vector_body + scalar_epilogue`) bloats the call-site.

### (c) "Pad columns to a multiple of W with sentinels"

- **Used by**: nobody in any of the four engines.
- **Pros**: every chunk is `&[T; W]`-shaped; no tail logic anywhere.
- **Cons**: dead memory at archetype tail (W-1 rows wasted per archetype);
  cache-miss on padding when streaming reads exceed live row count;
  sentinels must be representable as valid `T` (problematic for
  `T: !Default`); reads in `acc.algebraic_add(p[i].x)` would silently
  consume sentinel values, corrupting the reduction. **Fatal: silently
  changes the meaning of "sum of components".**

**flecs verdict (Sander Mertens,
[Building an ECS #2](https://ajmmertens.medium.com/building-an-ecs-2-archetypes-and-vectorization-fe21690805f9))**:
SoA per archetype with naturally-sized columns + user inner loop —
strategy (a). This is the documented design.

**Unity DOTS verdict**: ArchetypeChunk is a **fixed 16 KiB block** that
holds however many entities fit (e.g.,
`16384 / sizeof(Entity + Components)`). Tail is per-chunk: `chunk.Count`
is the live count; allocated capacity is `chunk.Capacity` (≥ Count).
User's inner loop runs `< chunk.Count`. Strategy (a) at the chunk
granularity, with an additional outer "block size" abstraction.

---

## 4. Closure-per-chunk vs iterator-of-chunks

### Iterator-of-chunks shape

```rust
impl<'q, 's, D: QueryData, F: QueryFilter> Query<'w, 's, D, F> {
    pub fn iter_chunks(&self) -> QueryChunkIter<'_, 'q, 's, D, F>;
}
impl Iterator for QueryChunkIter<'_, ...> {
    type Item = ChunkSlice<'_, D>;  // wraps &[T] or (&[A], &[B], ...)
    fn next(&mut self) -> Option<Self::Item>;
}
```

Pros: composes with `Iterator::filter` / `map` / `fold` / `flat_map`;
user can `break`/`continue`; matches Bevy idiom.

Cons:

- The `Iterator::next` state machine is harder to inline than a flat
  `for` loop. With multi-archetype walks, the cursor has to remember
  `(current_archetype, current_archetype_done)`.
- Lifetime gymnastics for the yielded slice — every `Item` must outlive
  `&mut self.next()`, which collides with the borrow on the QueryIter.
  Often forces a `Streaming Iterator` shape
  (`fn next<'a>(&'a mut self) -> Option<&'a T>`), which is NOT `Iterator`
  and can't compose.

### Closure-per-chunk shape (flecs `each`, Unity DOTS `Execute`)

```rust
impl<'q, 's, D: QueryData, F: QueryFilter> Query<'w, 's, D, F> {
    pub fn for_each_chunk<Func>(&mut self, mut f: Func)
    where
        Func: for<'c> FnMut(D::ChunkItem<'c>);
}
```

where `D::ChunkItem<'c> = &'c [T]` for `D = &T`, `&'c mut [T]` for
`D = &mut T`, tuples for tuple `D`.

Pros:

- No streaming-iterator problem — `'c` is a fresh per-call lifetime,
  scoped to the closure body.
- The closure is `FnMut`, naturally enclosing local state (accumulators).
- LLVM inlines `FnMut` through the static-dispatch generic parameter;
  benchmark behaviour matches a hand-written loop.
- Matches the proven shape from flecs and Unity DOTS — both production
  engines.

Cons:

- No `break`/`continue` from inside the closure unless we add a
  `ControlFlow` return.
- Composability with other adaptors requires writing a separate
  `fold_chunks` / `collect_chunks` API.

### Performance: does `impl Iterator<Item = &[T]>` cost more than `for_each_chunk(FnMut)`?

The Bevy PR [#6773 thread](https://github.com/bevyengine/bevy/pull/6773)
is the canonical Rust-ECS-specific source. The findings:

- The original `Query::for_each(FnMut)` was measurably faster than
  `Query::iter().for_each(...)` by 1.5–2× on certain queries pre-2023 —
  the gap being the `Iterator::next` state machine.
- PR #6773 ported the perf gain by **overriding `Iterator::for_each`** on
  `QueryIter` to call the same flat loop the standalone `for_each` used.
  Result: iterator-form now matches closure-form on `for_each` calls.
- For `fold`/`sum`/`reduce`: the standard `Iterator::fold` provided by std
  calls `next` in a loop, so the state machine reappears. To get parity,
  Bevy ALSO overrides `Iterator::fold` (the same PR + follow-ups).

**Conclusion for boyko**: a `for_each_chunk(FnMut)` shape is **the safer
floor**: it eliminates the streaming-iterator lifetime puzzle, matches
industry shape, and avoids needing to override every `Iterator` method to
recover perf. An `iter_chunks()` builder can be added later as a thin
wrapper — but the for_each shape is the load-bearing one.

---

## 5. Multi-component chunked queries

### flecs (C) — parallel arrays via repeated `ecs_field` calls

```c
while (ecs_query_next(&it)) {
    Position *p = ecs_field(&it, Position, 0);
    Velocity *v = ecs_field(&it, Velocity, 1);
    // p, v are both length it.count
    for (int i = 0; i < it.count; i++) {
        p[i].x += v[i].x * dt;
    }
}
```

- Same-length guarantee comes from the archetype invariant: every
  component in a table has identical row count.
- `ecs_field` returns `T*` typed against the macro's type argument;
  runtime size check via `ecs_field_w_size`.

### Unity DOTS — per-handle `NativeArray<T>`

```csharp
NativeArray<VelocityVector> velocityVectors = chunk.GetNativeArray(ref VelocityTypeHandle);
NativeArray<ObjectPosition> translations  = chunk.GetNativeArray(ref PositionTypeHandle);
```

- `NativeArray<T>` is a thin slice wrapper (`T*` + `length` + safety
  handle). Length == `chunk.Capacity` (≥ `chunk.Count`).
- Same-length per chunk by archetype invariant.

### `&mut` aliasing prevention at chunk level

Rust adds a constraint neither flecs nor DOTS faces: `&mut [B]` aliasing
rules.

For `Query<(&A, &mut B)>::for_each_chunk(|a: &[A], b: &mut [B]| ...)`:

- A ≠ B (different ComponentId) → no aliasing within the closure. boyko's
  existing `QueryData::init_access` registers `Read(A) + Write(B)` in
  `FilteredAccessSet`, raising `boyko-B0002` at system-registration time
  if conflicts exist.
- Cross-system: the Phase 9 scheduler's `ConflictGraph` already separates
  `(Read A, Write B)` from `(Write A, ...)`. No change needed.
- Cross-archetype within a single iteration: the closure gets one fresh
  `(&[A], &mut [B])` per archetype; each archetype's columns are disjoint
  memory. No overlap.

The architect should ensure the **closure signature uses non-aliasing
slice lifetimes per call**:

```rust
F: for<'c> FnMut(&'c [A], &'c mut [B])
```

so the borrow checker re-issues fresh borrows on each invocation.

---

## 6. Filter composition — the hard part

### flecs filter composition with batched iter

Recapping from §1.1:

- `Not<T>` — `ecs_field(it, T, term_index)` returns NULL for `Not` terms.
  Filter doesn't degrade the inner loop.
- `Optional<T>` — `ecs_field_is_set(it, term_index)` returns a `bool`.
  User branches per-table (not per-row) on whether the optional column is
  present. Slice-shape preserved.
- Archetypal `With<T>`/`Without<T>` (boyko's `IS_ARCHETYPAL = true` filter
  category) — affect WHICH archetypes are visited, not what happens
  inside. Slice-shape preserved.

### Per-row tick-based filters (`Changed<T>`, `Added<T>`)

This is the structural blocker for "slice + filter" in Bevy/boyko-style
change-detection ECS.

**Bevy's per-row tick storage** (PR
[#6547, merged 2022-11-21](https://github.com/bevyengine/bevy/pull/6547)
and successors): each component column has a parallel
`Box<[UnsafeCell<Tick>]>` of "added ticks" and "changed ticks".
`Changed<T>` filter calls `Tick::is_newer_than(last_run, this_run)` per row.

**boyko's Phase 10 change detection** (per project memory): identical
model — `Box<[UnsafeCell<Tick>]>` per-row. Phase 12.5 NCD6 already
const-folds away the tick-load when no filter/data uses ticks
(`if const { D::NCD || F::NCD }`).

The fundamental constraint: with a per-row tick + per-row include/exclude
decision, a `&[T]` slice **cannot represent the filtered output without
either (a) yielding a discontiguous subset, or (b) including all rows and
providing the mask separately**.

Three viable behaviours, **all of which appear in production engines or
were considered for them**:

#### Option A — "compile-time refuse filters that need per-row decisions"

```rust
impl<'w, 's, D: QueryData> Query<'w, 's, D, /* F = no per-row filter */> {
    pub fn for_each_chunk<Func>(&mut self, f: Func) where ...;
}
```

- Gate via a marker trait: `F: ArchetypalFilter` (boyko's existing
  `IS_ARCHETYPAL` const flag, see `QueryFilter` impls). `Changed<T>` does
  NOT satisfy `IS_ARCHETYPAL`, so the call to `for_each_chunk` won't
  typecheck.
- **Precedent**: this is what Bevy PR #6161 implicitly did — the
  `for_each_mut_batched` API never landed support for `Changed<T>`.
  Bevy's `Query::iter().for_each` keeps the per-row branch; users wanting
  batched + change-detect have to drop down to scalar.
- **Pro**: zero performance compromise; no scalar fallback.
- **Con**: users with `Changed<T>` must keep calling `iter()`;
  discoverability cost.

#### Option B — "yield slice + precomputed mask"

```rust
pub fn for_each_chunk_filtered<Func>(&mut self, f: Func)
where
    Func: for<'c> FnMut(&'c [T], &'c BitSet);
```

- Engine scans the per-row ticks once per archetype, builds a `BitSet512`
  (boyko has this!), passes both to the closure.
- User chooses: scalar walk over set bits, OR SIMD over the whole slice
  and let the mask gate the writes (using `_mm256_blendv_ps` etc.) — the
  latter is sometimes faster than the scalar walk when most rows are
  changed.
- **Precedent**: Unity DOTS does exactly this with `v128 chunkEnabledMask`
  (see §1.4). The DOTS `ChunkEntityEnumerator` is a wrapper over a
  bitmask scan.
- **Pro**: composes correctly with all filters; user-controlled vector
  width.
- **Con**: cost of building the mask is now in the engine; if the mask
  is rarely consulted the cost is wasted.

#### Option C — "scalar fallback"

```rust
pub fn for_each_chunk<Func>(&mut self, f: Func) {
    if F::IS_ARCHETYPAL {
        // fast path: per-archetype slice
    } else {
        // fallback: per-row iter, calling f with a 1-element slice each time
    }
}
```

- **Pro**: one API; works in all cases.
- **Con**: silent perf cliff; degrades the very thing the API is meant
  to enable. User has no signal that they're on the slow path.

**Inference for the architect**: the cleanest separation is **Option A as
the default + Option B as an opt-in `for_each_chunk_with_mask` variant**.
Option C is a footgun.

---

## 7. Parallel composition

### flecs

`ecs_worker_iter(it, worker_idx, worker_count)` (per
[Iterators API](https://www.flecs.dev/flecs/group__iterator.html)): splits
the matched entities across N workers. Each worker drives its own
`while (ecs_worker_next(&it_slice))`. Distribution is at the entity-count
granularity, not the archetype boundary — flecs explicitly splits
archetypes if needed to balance work.

flecs has no `par_for_each_chunk` primitive in the public C API; the
system pipeline runs systems in parallel, and each system internally uses
`ecs_worker_iter`.

### Unity DOTS

`IJobChunk.ScheduleParallel(query, dependsOn)`: the job system fans
**whole chunks** (not chunk-subranges) across worker threads. Each chunk
is a fixed-size block (16 KiB), so dispatch granularity is "an even
number of KiB of components" per worker.

### boyko-engine — existing model (Phase 9)

`Query::par_iter` already fans per-archetype subranges across workers via
`ThreadPool::scope`, with `MIN_ARCHETYPE_FOR_PARALLEL = 1024` inline
threshold. The chunked API can layer cleanly:

```rust
impl<'w, 's, D: QueryData, F: QueryFilter> Query<'w, 's, D, F> {
    pub fn par_for_each_chunk<Func>(&mut self, f: Func)
    where
        Func: for<'c> Fn(D::ChunkItem<'c>) + Send + Sync;
}
```

The split-granularity question:

- **Per-archetype**: each worker gets whole archetypes. Smallest dispatch,
  best for big archetypes; idle workers when archetype count < worker
  count.
- **Per-subrange (Phase 9 default)**: each worker gets a row range within
  one archetype. Higher dispatch cost; better load balance.

Cost-benefit:

- A scope.spawn costs ~120 ns (per boyko Phase 9 plan §10.3).
- For batched SIMD ops, the inner work per row is on the order of
  0.1–1 ns (AVX2 8-wide f32 ops at ~2 cycles).
- An archetype of 1024 rows × 1 ns = 1 μs of work, vs 120 ns dispatch =
  12% overhead. **Per-archetype is the cleaner choice for the chunked
  API** — each worker gets `(start_row, end_row)` and calls
  `f(&slice[start..end])`, giving the closure the natural slice.

The Phase 9 `BatchingStrategy` already exists and parameterizes the
split. `par_for_each_chunk` can re-use it.

---

## 8. Naming / ergonomics

Comparing the established names:

| Name | Source | Connotation |
|---|---|---|
| `for_each` | Rust std `Iterator::for_each` | per-row, no chunk |
| `for_each_chunk` | proposed | clear "chunk = engine-defined unit"; collides nameishly with `slice::chunks` |
| `for_each_archetype_slice` | descriptive | accurate; very long |
| `iter_chunks` | proposed | implies returning an Iterator (which §4 argued against as primary) |
| `for_each_table` | Bevy internal naming | confuses storage type (Table vs SparseSet); boyko has only one storage so this is misleading |
| `each_iter` | flecs Rust binding | low Rust-idiomatic; cryptic |
| `for_each_batched` | Bevy PR #6161 | "batched" overloaded with `Commands::spawn_batch` |
| `run` | flecs C++ | too generic; no slice connotation |

**Rust-idiomatic candidates:**

- **`for_each_chunk`** — closest to flecs naming, parallels `slice::chunks`
  (which is widely known), and the `_chunk` suffix unambiguously implies
  "engine yields a chunk; you do the inner loop." This is the name in the
  Phase 13 roadmap.
- **`fold_chunks`** for a reducing variant — composes with the
  `Iterator::fold` mental model.
- **`par_for_each_chunk`** for the parallel form — symmetric with the
  existing `par_iter` / `par_iter_mut`.

Discoverability check: `Query::for_each_chunk` is googleable as a
Bevy-style Rust ECS name; `Query::each_iter` is not.

Collision check with `core::slice::chunks`: the conceptual overlap is
intentional — both mean "engine-controlled subdivision of a larger
sequence." No name collision in the type system (different types).

---

## 9. Comparative API-shape recommendation table (for architect)

| Aspect | flecs (C) | flecs (C++ `run`) | Bevy PR #6161 (closed) | Unity DOTS | **Inferred floor for boyko** |
|---|---|---|---|---|---|
| Shape | drive loop yourself | drive loop yourself | dual closure | closure per chunk | closure per chunk |
| Per-call yield | `T*` + `count` | `flecs::iter` | `(scalar_item, batched_item)` | `ArchetypeChunk` | `&[T]` or tuple |
| Tail strategy | (a) user-handled | (a) user-handled | (b) engine pre-chunks aligned | (a) per-block | (a) user-handled |
| Lane-width generic | none | none | `::<N>` | none | none |
| Alignment generic | none | none | `Align16` / `Align32` | none | none (column-start only) |
| Filter w/ per-row | `Optional` requires `is_set` | same | not addressed | `v128` mask | **Option A**: refuse non-archetypal F at compile time |
| Multi-component | `ecs_field` per term | `it.field<T>(i)` | tuple of slices | `chunk.GetNativeArray<T>` | tuple of slices |
| Mutable safety | unchecked | unchecked | borrow-checked | safety handle | borrow-checked, per-call fresh `'c` |
| Parallel pair | `ecs_worker_iter` | same | not present | `ScheduleParallel` | `par_for_each_chunk` reusing Phase 9 |

---

## 10. Recommendation summary (input to architect, not final)

(Architect: this is the researcher's distilled mapping of constraints to
options. Decisions remain yours.)

**For the API surface itself:**

- Name: `Query::for_each_chunk` + `Query::par_for_each_chunk`
  (Rust-idiomatic, matches Phase 13 roadmap, googleable).
- Shape: **closure-per-chunk** with
  `F: for<'c> FnMut(D::ChunkItem<'c>)` (or `Fn + Send + Sync` for the
  parallel form).
- `D::ChunkItem<'c>`:
  - `&T` → `&'c [T]`
  - `&mut T` → `&'c mut [T]`
  - tuples → tuples of element `ChunkItem`s, all same length per archetype
    (guaranteed by archetype invariant).
- Tail handling: strategy (a) — engine yields one slice per archetype;
  user writes `chunks_exact(W) + remainder()`. This is what flecs does
  and what won't burn the API budget on alignment generics.

**For filter composition:**

- `for_each_chunk` is **gated on `F: ArchetypalFilter`** at compile time
  (use boyko's existing `IS_ARCHETYPAL` const). `Changed<T>` / `Added<T>`
  won't typecheck — users keep `iter()`/`iter_mut()` for those.
- Phase 13+ may add `for_each_chunk_with_mask` as a separate entry point
  that yields `(slice, &BitSet)`, mirroring Unity DOTS. Defer.

**For alignment:**

- Promise only **column-start alignment** (e.g., 32-byte for AVX2
  baseline). Per-row alignment is `align_of::<T>()` and not
  engine-controlled. This sidesteps the Bevy #6161 Vec3-padding
  soundness hole.
- Verify `ComponentPool::buffer_ptr()` is 32-byte aligned today (or add
  an Arena layer alignment assertion if not).

**For the bench harness:**

- Change `acc += black_box(p.x)` →
  `acc = f32::algebraic_add(acc, p.x)` inside the closure body, OR
  `acc = slice.iter().copied().fold(0.0, f32::algebraic_add)` outside
  the closure.
- Requires `#![feature(float_algebraic)]` (`rust-lang/rust#136469` —
  still nightly as of May 2026). boyko's bench crates use nightly today
  (verify).
- For a stable-Rust bench: use
  `std::arch::x86_64::{_mm256_loadu_ps, _mm256_add_ps, _mm256_hadd_ps}`
  AVX2 intrinsics — boyko's stated baseline target.
- Expected speedup floor on the chunked-vs-scalar comparison: **5–20×**
  for f32 reductions (per orlp.net 21.6× and Bevy PR #6547 1.36–2.06× on
  busy_systems).

**For the parallel variant:**

- Re-use Phase 9 `BatchingStrategy` and `ThreadPool::scope`. Each worker
  receives `(archetype_id, row_range)` and calls
  `f(&column_slice[row_range])`. Same `MIN_ARCHETYPE_FOR_PARALLEL = 1024`
  threshold.

**Open questions for the architect:**

1. Should `D::ChunkItem<'c>` be a new GAT on `QueryData`, or a new
   sibling trait `ChunkedQueryData`? The former touches every QueryData
   impl (78 impls per Phase 10 history); the latter is opt-in but creates
   two parallel hierarchies.
2. Confirm column-start alignment story: does Arena guarantee 32-byte
   alignment for `ComponentPool::buffer_ptr()` today, or does Phase X.A
   need to lift that as a precondition?
3. Should there be a `Query::for_each_chunk_mut` (mirror of `iter_mut`)
   explicit, or is the read-vs-write split type-inferred from `D`'s
   contents? boyko's current pattern is type-inferred — keep it.
4. Bench harness: stable (std::arch intrinsics) vs nightly
   (`float_algebraic`)? The latter is more portable across CPUs but pins
   boyko's bench to nightly. boyko's stance per CLAUDE.md is nightly is
   acceptable for measurable gains.

---

## 11. Sources

[1] [flecs Queries documentation, master branch](https://github.com/SanderMertens/flecs/blob/master/docs/Queries.md) — canonical C/C++ API for `ecs_query_next + ecs_field + run + each`. Quoted code blocks.

[2] [flecs Iterators API reference](https://www.flecs.dev/flecs/group__iterator.html) — `ecs_field_w_size`, `ecs_worker_iter`, `ecs_page_iter` signatures.

[3] [Sander Mertens — Building an ECS #2: Archetypes and Vectorization](https://ajmmertens.medium.com/building-an-ecs-2-archetypes-and-vectorization-fe21690805f9) — design rationale for SoA + columnar slice + user inner loop, by the flecs author.

[4] [Bevy PR #6161 "Implement batched query support" by InBetweenNames](https://github.com/bevyengine/bevy/pull/6161) — **CLOSED 2024-10-06 inactivity, `S-Adopt-Me`**. The only serious Rust ECS chunked-iter attempt. Critical reviewer feedback on alignment + `Vec3` padding soundness.

[5] [Bevy issue #1990 "Batched ECS Query"](https://github.com/bevyengine/bevy/issues/1990) — **OPEN since 2021**, no assignee, `S-Adopt-Me`.

[6] [Bevy PR #6773 "Override QueryIter::fold ..."](https://github.com/bevyengine/bevy/pull/6773) — merged 2023-12-01. Documents the `Iterator::fold` override approach that landed in Bevy as an alternative to a batched API.

[7] [Bevy PR #6547 "Split component ticks"](https://github.com/bevyengine/bevy/pull/6547) — merged 2022-11-21. 1.36–2.06× busy_systems speedup; demonstrates the autovec gain Bevy unlocked by splitting tick storage to enable LLVM vectorization on `iter()`.

[8] [Bevy current `crates/bevy_ecs/src/query/iter.rs`](https://github.com/bevyengine/bevy/blob/main/crates/bevy_ecs/src/query/iter.rs) — current `fold_over_storage_range`, `fold_over_table_range`, `fold_over_archetype_range`, `fold_over_dense_archetype_range` signatures.

[9] [EnTT issue #462 "Iteration over continuous intervals of components"](https://github.com/skypjack/entt/issues/462) — CLOSED. Closest analog to boyko's proposed API; the EnTT author chose not to ship it.

[10] [EnTT Crash Course: entity-component system](https://github.com/skypjack/entt/wiki/Crash-Course:-entity-component-system) — `group.raw<T>()` documented as the manual-slice escape hatch.

[11] [Unity DOTS Entities 1.0 — Implementing IJobChunk](https://docs.unity3d.com/Packages/com.unity.entities@1.0/manual/iterating-data-ijobchunk-implement.html) — full `Execute` example, `ChunkEntityEnumerator`, `chunkEnabledMask: v128`.

[12] [Unity DOTS Entities 1.0 — IJobChunk interface reference](https://docs.unity3d.com/Packages/com.unity.entities@1.0/api/Unity.Entities.IJobChunk.html) — exact `Execute` signature.

[13] [orlp.net — Taming Floating-Point Sums](https://orlp.net/blog/taming-float-sums/) — the 21.6× speedup data (5.5 → 118.6 GB/s) on AMD Threadripper 2950X for `fadd_algebraic`-based f32 sum of 100,000 elements.

[14] [orlp/sum-bench repository](https://github.com/orlp/sum-bench) — reproducible benchmark accompanying the blog post.

[15] [Rust tracking issue rust-lang/rust#136469 — algebraic float methods](https://github.com/rust-lang/rust/issues/136469) — `f32::algebraic_add` stabilization, **status: FCP proposed-final-comment-period, disposition-merge, no stabilization PR landed as of May 2026**.

[16] [Rust tracking issue rust-lang/rust#86656 — portable SIMD (RFC 2948)](https://github.com/rust-lang/rust/issues/86656) — `std::simd` still nightly-only; long-term stabilization timeline unclear.

[17] [std::intrinsics::fadd_algebraic documentation](https://doc.rust-lang.org/std/intrinsics/fn.fadd_algebraic.html) — exact signature, `#![feature(core_intrinsics)]` gate.

[18] [Sergey "Shnatsel" Davidoff — The state of SIMD in Rust in 2025](https://shnatsel.medium.com/the-state-of-simd-in-rust-in-2025-32c263e5f53d) — confirms `std::simd` and `float_algebraic` still nightly; current best-practice survey (`wide`, `pulp`, `macerator`).

[19] [Nick Wilcox — Auto-Vectorization in Rust](https://www.nickwilcox.com/blog/autovec/) — slice + `chunks_exact_mut` + `zip` pattern; bounds-check elision technique.

[20] [Nick Wilcox — Auto-Vectorization for Newer Instruction Sets](https://www.nickwilcox.com/blog/autovec2/) — runtime feature detection pattern; ~2× AVX2 vs SSE2 baseline.

[21] [Intel Community — vmovups vs vmovapd performance implications](https://community.intel.com/t5/Intel-ISA-Extensions/what-are-the-performance-implications-of-using-vmovups-and/m-p/1143448) — cross-cache-line load penalty 2× (128-bit) / 4× (256-bit); aligned-vs-unaligned penalty within a cache line is zero on modern uarchs.

[22] [Bevy `QueryParIter` docs](https://docs.rs/bevy/latest/bevy/ecs/query/struct.QueryParIter.html) — Bevy's parallel iter shape for cross-reference.

[23] [Bevy `QueryIter` docs](https://docs.rs/bevy/latest/bevy/ecs/query/struct.QueryIter.html) — current public Iterator API.

[24] [Unofficial Bevy Cheat Book — Internal Parallelism](https://bevy-cheatbook.github.io/programming/par-iter.html) — Bevy's parallelism conceptual overview.

[25] [The Rust Performance Book — Auto-vectorization section](https://nnethercote.github.io/perf-book/) — general guidance on LLVM autovec triggers.

[26] **Local boyko codebase**:

- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\iter.rs` — current `QueryIter` / `QueryIterMut` cursor shape; two-level `loop { while { ... } }` with `IS_ARCHETYPAL` const-fold (the `for_each_chunk` API will need to mirror the `set_table_readonly` / `set_table_mut` split and the NCD6 const-fold).
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\par_iter.rs` — existing `ParQuery::for_each` / `BatchingStrategy` / `MIN_ARCHETYPE_FOR_PARALLEL = 1024` — direct reuse target for `par_for_each_chunk`.
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\data.rs` — `QueryData` trait with `Fetch<'w>` GAT, `set_table_{readonly,mut}{,_no_meta}` methods, `IS_READ_ONLY` + `NEEDS_CHANGE_DETECTION` consts; the `for_each_chunk` GAT extension lives here.
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\query.rs` — `Query<'w, 's, D, F>` struct; the new method goes here.
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\memory\component_pool.rs:732` — `ComponentPool::buffer_ptr()` exists; alignment guarantee needs verification.
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\archetype\archetype.rs:345` — `Archetype::entity_count()` exists; gives the slice length.
- `D:\claude\BoykoEngine\docs\PHASE-13-ROADMAP.md` — Phase X.A line items (lines 80–103) define the agreed scope.
