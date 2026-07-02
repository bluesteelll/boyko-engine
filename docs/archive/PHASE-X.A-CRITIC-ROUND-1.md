> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase X.A — Architecture Critic, Round 1

**Verdict:** APPROVED WITH MINOR CHANGES — the plan is structurally
sound. The big-ticket decisions (sibling trait, marker filter,
alignment lift, NCD elision via type bounds, per-archetype-subrange
parallel granularity) are well-justified and consistent with the
existing code. The five W-tier remarks are scope-clarifying patches
the developer should fold in before Step 1A starts, not architectural
overhauls. No critical (C-tier) blockers.

The path checking confirmed:

- 32-B aligned column starts propagate from `ComponentPool::new`
  through `Column.ptr` to `buffer_ptr()` (one place to lift).
- `Or<F>` propagation composes because the real impl is
  `Or<(F0..)>` and the tuple bound forces every element archetypal.
- `EcsMaster::query`'s `const { D::NCD || F::NCD }` panic is
  unreachable at chunked monomorphisations.

---

## Remarks (priority-ordered)

### W1 — `#[inline]` table in §10.7 self-contradicts on `for_each_chunk` outer body

**Section:** §10.7, plan lines 839 and 843.

Line 839 (row "Query::for_each_chunk outer body") says **NO
annotation**. Line 843 (row "for_each_chunk outer-loop entry") says
**#[inline]** with the rationale "the method is cross-crate; users
call it from system bodies in their own crate; LTO is opportunistic
if `#[inline]` is omitted". These two rows describe the same function
and disagree.

The existing Phase 9 precedent is unambiguous:
`ParQuery::for_each` / `ParQueryMut::for_each` (the public shim) at
`par_iter.rs:166-167` and `par_iter.rs:216-217` ARE `#[inline]`; the
internal `for_each_impl` driver at `par_iter.rs:244` is NOT. The
chunked plan should mirror this exactly: `#[inline]` on the
`Query::for_each_chunk` / `Query::par_for_each_chunk` public methods
(they cross the user's crate boundary and need LTO visibility for
closure inlining), no annotation on the internal
`chunk_iter::for_each_chunk_impl` / `par_chunk::par_for_each_chunk_impl`
drivers.

**Suggestion:** collapse the two rows into one — `Query::for_each_chunk`
public method: `#[inline]` (cross-crate visibility);
`chunk_iter::for_each_chunk_impl` driver: no annotation (LLVM decides;
mirrors `par_iter.rs:244`). Same for `par_for_each_chunk` /
`par_for_each_chunk_impl`.

---

### W2 — Sequential-vs-parallel closure semantics drift is a footgun the plan doesn't surface

**Section:** §2.3 vs §2.4, §9.2.

Sequential `for_each_chunk` calls the closure **once per archetype**
(Step 4 driver, plan lines 1093-1094:
`fetch_chunk(&chunk_fetch, 0, entity_count)`). The user's reduction
(`acc += slice.iter().sum()`) gets a single full slice per archetype.

Parallel `par_for_each_chunk` calls the closure **once per
`(start, end)` sub-range**. A 100k-row archetype yields ~100 calls
× 1000 rows (per the BatchingStrategy default). The user's
`Fn(&[T]) + Send + Sync` accumulator pattern requires interior
mutability (`AtomicF32`?), a sharded thread-local, or a deferred
fold. Sequential `FnMut(&mut f32 capture, &[T])` doesn't translate.

This is fine architecturally — same shape as Bevy's `par_iter` vs
`iter` divergence — but the plan never tells the user. §2.6 defers
`fold_chunks` to Phase 13.X for "the parallel reducing variant
requires its own design", which IS the right deferral, but the user
reading just §2.3/§2.4 won't realize that the canonical f32-sum
reduction (§8.2 bench shape) **cannot be expressed in
`par_for_each_chunk` with a clean closure**.

**Suggestion:** add a sentence to §2.4 docs (and the rustdoc on
`par_for_each_chunk`): "The closure is invoked once per archetype
sub-range, not once per archetype. For reductions, use a thread-safe
accumulator (e.g., a sharded `[AtomicF32; N]`) or wait for
`par_fold_chunks` (Phase 13.X)." Also: add a `par_for_each_chunk`
row to the §1.2 table clarifying the per-call frequency target —
currently §1.2 only states "dispatch overhead per archetype-chunk
≤ 150 ns", which doesn't quantify the user-visible-call frequency.

---

### W3 — `rust-toolchain.toml` at workspace root forces nightly on the engine library, not just the bench

**Section:** §8.1.

`rust-toolchain.toml` at `D:\claude\BoykoEngine\rust-toolchain.toml`
(workspace root) applies to **every** `cargo` invocation in the
workspace, including `cargo check`, `cargo test`, `cargo clippy` on
`boyko_ecs` itself. The plan acknowledges this in §8.1 ("the engine
crate does not gain any nightly-only features") but the side effect
is broader: CI workflows that pin to stable (e.g., the `docs.yml`
GitHub Pages deploy that builds rustdoc) will silently start using
nightly. Phase 12.5 memory notes "Engine — preferably stable; benches
и SIMD-критичные пути — nightly OK" — this contradicts that policy.

The plan §8.5 mentions a stable fallback "if the orchestrator later
vetoes nightly" but treats it as a contingency rather than the safer
default. The more conservative shape is:

- No workspace-level `rust-toolchain.toml`.
- Bench-crate-local `rust-toolchain.toml` at
  `crates/bench_bevy_vs_boyko/rust-toolchain.toml`, OR
- `cargo +nightly bench --bench g6_for_each_chunk` invoked
  explicitly in the bench step (no workspace pin).

The bench is the only crate that needs nightly
(`#![feature(float_algebraic)]`). Confining the pin keeps the engine
and the doctest path on stable.

**Suggestion:** move the toolchain pin into
`crates/bench_bevy_vs_boyko/` (Cargo supports per-package
`rust-toolchain.toml` — verify against rustup docs), or document
`cargo +nightly bench ...` as the canonical invocation and skip the
file entirely. Either way, the engine's `cargo check --all-targets`
should remain stable-Rust-compatible.

---

### W4 — `Box<UnsafeCell<...>>` cache slot allocation cost vs §1.2's "0 allocations per frame"

**Section:** §1.2 "Allocations per frame on hot path: 0".

§1.2 claims zero allocations per frame. The chunked path itself
allocates nothing — true. But `Query::for_each_chunk` and
`QueryView::for_each_chunk` use the **existing
`query_state_cache`** which on **first use of a new `(D, F)`
pair** invokes `query_cold_init` (plan-referenced
`ecs_master.rs:1949`). That function does
`let cell = Box::new(UnsafeCell::new(state));` plus `Box::leak`. So:

- Steady state (post-warmup): 0 allocations. The §1.2 claim holds.
- First call to `for_each_chunk::<NewD, NewF>` in a system: 1 `Box`
  allocation + `OnceLock::set`.

The architect's NCD elision means `(D, F)` for the chunked path is
`(&Position, ())` or similar — concretely one new `(D, F)` per call
site. If the user adds a `for_each_chunk` call to a system that
already had `iter()` over the same `(D, F)`, the cache slot is
reused (cache key is `(D, F)`, not API-flavoured). Good.

But if a user writes `Query<&A>::iter()` AND
`Query<(&A, &B)>::for_each_chunk()` in the same system, they pay
two cold allocations across the system's first frame. This is fine —
same cost model as today — but the §1.2 row reads as if the chunked
path is allocation-free in absolute terms, which is misleading on
cold paths.

**Suggestion:** edit the §1.2 "Allocations per frame on hot path"
row to read "0 in steady state; one `Box<UnsafeCell<QueryDataState<D, F>>>`
per **new** `(D, F)` pair on first use (same shape as Phase 12.6
`EcsMaster::query` direct API; cached for the world's lifetime)."
This matches the cost model the developer needs to honor.

---

### W5 — Risk register glosses the `Drop` order of column buffers on the alignment lift

**Section:** §13 Risk 4 + §6.2.

Lifting `ComponentPool` buffer alignment from `align_of::<T>()` to
`max(align_of::<T>(), 32)` changes the `Layout` passed to
`Arena::allocate_layout`. Risk 4 covers the allocation path. The
unspoken side is the `dealloc` path: the arena's
`MemFreeBlockMaster` records `(start, size)` free blocks (not
`(start, size, align)`); when a `ComponentPool` is destroyed and its
buffer returns to the free list, the alignment metadata is dropped.
The next allocator request for, say, an `f64`-sized column (8-byte
align) might land at an address that happens to satisfy 8 but not 32.
That's fine — the next allocation will use its own `align` parameter.

But there is a subtler concern in `Arena::Drop` (arena.rs:189-196):
the original `Layout` passed to `dealloc` is the **arena-level**
layout (`CACHE_LINE_SIZE` = 64), not the per-buffer alignment. So
the arena's `dealloc` is unaffected by the lift. No issue here.

The actual residual risk is `MemFreeBlockMaster::allocate_aligned`
(referenced at arena.rs:161 and in plan §13.4). The plan asserts
this "supports arbitrary power-of-2 alignment via best-fit +
alignment-up" without quoting an existing test that exercises
`align = 32` from a `ComponentPool::new` call site. The existing
`arena_allocate_typed_returns_correct_alignment` test (plan
reference: arena.rs:301) tests via `#[repr(align(32))] struct Fat` —
that's the arena layer, not the pool layer. The chain
`ComponentPool::new → arena.allocate_layout(buffer_layout, align=32)
→ MemFreeBlockMaster::allocate_aligned(_, 32)` has no direct
integration test.

**Suggestion:** Wave 1A's unit test `buffer_ptr_is_simd_aligned`
(Step 1A, plan line 976) — make it the **first** test the developer
writes, before any chunked-API code. If it fails, the alignment lift
is broken at the arena layer and the rest of the wave is meaningless.
The plan currently lists it last in the Step 1A bullet list;
reorder so it's the gating test for the entire wave. (No
architectural change — purely procedural.)

---

## Nitpicks (N-tier — informational, not required)

### N1 — §1.2 "≤ 256 B L1i footprint" target is unfalsifiable without a measurement plan

The §1.2 row "L1i footprint of `for_each_chunk` per-archetype dispatch
body ≤ ~256 B (rough; verify via `cargo asm`)" lists no concrete
asm-inspection step in the Wave 8 bench plan. §11.6 only mentions
`cargo asm` for the inner-loop autovec verification. If the I-cache
budget matters, add a one-line asm-size check in §11.6. Otherwise
drop the target — it adds nothing.

### N2 — §5.2 wording "Or<F> propagation" is technically correct but the inner-tuple detail belongs in the SAFETY comment

The blanket `unsafe impl<F: ArchetypalQueryFilter> ArchetypalQueryFilter
for Or<F> {}` works only because the actual `Or<F>` impl in
`filter.rs:1151` is `Or<(F0, F1, …)>` and the tuple `(F0, F1, …)` is
`ArchetypalQueryFilter` iff every element is. The plan §5.2 line 458
acknowledges this in prose ("the tuple impl above ensures
`F = (F0, F1)` is `ArchetypalQueryFilter` iff `F0, F1` both are").
Worth adding the same observation as a `// SAFETY:` rationale on the
`Or<F>` impl itself so a future contributor reading just the impl
block sees why the bound `F: ArchetypalQueryFilter` is sufficient.

### N3 — §6.2 cost analysis cites "512 components × 1024 archetypes" pathological case but the bound is `MAX_COMPONENTS`

The "14 MB wasted out of 64 MB" worst-case estimate in §6.2 uses
512 components × 1024 archetypes. Cross-check: `MAX_COMPONENTS`
constant in the codebase. If it's actually 512, the estimate stands;
if not, the number is wrong. Either way, this is a "back of envelope"
— the conclusion ("acceptable") is right but the arithmetic should
be sourced from `constants.rs`.

### N4 — Step 7B compile-fail test for `Query<(&mut T, &mut T)>::for_each_chunk` won't fire via `EcsMaster::query` direct API

§11.2 last row ("aliasing_query_mut_t_mut_t_rejected.rs") needs to
exercise the **SystemParam** path (inside a system body), not
`world.query::<(&mut T, &mut T), ()>()`. The direct API bypasses
`FilteredAccessSet` entirely (verified: `ecs_master.rs:1886-1939`
shows `query_cold_init` only calls `QueryDataState::new` which calls
`init_state` not `init_access`). Make sure the trybuild test's setup
uses a `#[system]` fn or builds a `Schedule`, otherwise the test
passes the type-check and silently runs (UB!) rather than failing
to compile.

---

## Positive observations

- **Sibling-trait decision in §4 is correct.** The 78-impl risk
  surface argument is well-grounded — the actual count is 32 method
  bodies just in `data.rs` leaves, plus the tuple expansion. A
  `QueryData` GAT extension would force every downstream consumer to
  either add the new methods or break. Sibling trait is the right
  call and the architect's trade-off table accurately captures it.
- **§7 NCD elision via type bounds is elegant.** Using
  `D: ChunkedQueryData` + `F: ArchetypalQueryFilter` to make
  `const { D::NCD || F::NCD }` always-false at the chunked
  monomorphisation is cleaner than carrying `_no_meta` variants.
  This is a measurable I-cache win — one fewer fn-pointer-indirection
  per archetype boundary.
- **§6.5 explicit rejection of per-row alignment promises**
  correctly distances Phase X.A from the Bevy PR #6161 `Vec3`
  soundness blocker. The architect read the research and didn't fall
  into the same trap.
- **§9.4 dropping `meta` and `mutable` from `ChunkChunkCaptures`
  where statically derivable** is a small but real perf improvement
  vs blindly copying par_iter's shape. Smaller capture struct →
  smaller `move` closure → less per-spawn cost.
- **§13 risk register names each risk with a falsifiable trigger
  and a mitigation test** — including residual risk levels. This is
  the right shape for the developer's planning input.
- **No `#[inline(always)]` in the plan, anywhere.** The architect
  honored CLAUDE.md principle 7 throughout, including a self-aware
  "Critic-deflection note" at line 845. Preserve this.

---

## Open questions for the architect (none blocking; informational)

1. The §1.2 target "≤ 5 ns outer-loop per empty archetype" assumes
   the `entity_count == 0` continue branch fires before any
   `set_chunk_*` call. The plan's Step 4 driver (line 1075-1076)
   gets `entity_count = (*arch_ptr).entity_count()` BEFORE the
   `set_chunk_*` dispatch — good. Confirm the asm verification in
   §11.6 includes this empty-archetype path.
2. The §8.1 toolchain pin date "nightly-2026-05-01" is post-cutoff
   guessing. Worth deferring the exact date to Wave 8A and writing
   "latest stable nightly at impl time" in the plan to avoid baking
   in a date that may not exist.

---

## Files relevant to this review (absolute paths)

- `D:\claude\BoykoEngine\docs\PHASE-X.A-PLAN.md` (1344 lines —
  full plan, reviewed)
- `D:\claude\BoykoEngine\docs\PHASE-X.A-RESEARCH.md` (1018 lines —
  research input, spot-checked §2.1, §2.3, §2.5, §3, §4)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\filter.rs`
  (verified `Or<F>` is `Or<(F0..)>`, tuple-AND impl shape, 5 leaf
  `QueryFilter` impls)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\data.rs`
  (verified 32 `set_table_*` method bodies in leaves, 12 tuple
  invocations + 12 too-large, `init_state` / `init_access` split)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\par_iter.rs`
  (verified `for_each_impl` is the unannotated internal driver,
  `for_each` shim is `#[inline]`, `ChunkCaptures` Send shape,
  BatchingStrategy)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\iter.rs`
  (verified NCD6 const-fold dispatcher pattern at lines 244-269)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\query.rs`
  (verified `Query` struct shape, `iter` / `iter_mut` / `par_iter`
  signatures, `meta` field plumbing)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\query_view.rs`
  (verified `QueryView` does NOT carry `meta`, uses
  `SystemMeta::dummy()` for NCD7)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\memory\arena.rs`
  (verified 64-B aligned arena base, ALLOC1 guard on
  `allocate_layout` — chunked path doesn't allocate at runtime)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\memory\component_pool.rs`
  (verified §6 alignment lift target — `ComponentPool::new`
  line 120-122 is the single point to change)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\archetype\archetype.rs`
  (verified `Column.ptr = pool.buffer_ptr()` at line 301 — alignment
  lift propagates here)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\ecs_master\ecs_master.rs`
  (verified `EcsMaster::query`'s `const { D::NCD || F::NCD }` panic
  gate at line 1894, `query_cold_init` Box allocation at line 1955 —
  W4 cost-model context)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\state.rs`
  (verified `QueryDataState::new` calls `init_state`, NOT
  `init_access` — N4 trybuild caveat)
- `D:\claude\BoykoEngine\crates\bench_bevy_vs_boyko\Cargo.toml`
  (verified no current nightly use — W3 toolchain pin scope)
- `D:\claude\BoykoEngine\Cargo.toml` (verified workspace root
  structure for W3)
