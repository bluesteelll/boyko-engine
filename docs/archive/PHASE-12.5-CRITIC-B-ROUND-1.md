> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase 12.5 Track B — Critic Round 1

Critique of `docs/PHASE-12.5-QUERY-OPTIMIZATIONS-PLAN.md` (Round 1).
Verdict: **NEEDS-FIX**.

## CRITICAL (block APPROVED)

### C1. `EcsMaster: Send + Sync` — plan's QV1 / §7.4 / PHASE9.1 founding claim "EcsMaster: !Send + !Sync" is factually wrong, and the entire concurrency story collapses.

**Where**: plan §4.3 (lines 109, 488–494), §7.4 (lines 968–975), QV7, PHASE9.1.

**Why critical**: `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs:1501-1502`
ships `unsafe impl Send for EcsMaster {}` + `unsafe impl Sync for EcsMaster {}`,
asserted at compile time by `assert_send_sync::<EcsMaster>()`. Phase 9's
whole point (SEND1/SEND2/SEND3) is that workers receive `UnsafeEcsCell<'w>`
(which is `Copy + Send + Sync` per `unsafe_ecs_cell.rs:341-342`) and call
`cell.world() -> &'w EcsMaster` from worker threads.

Therefore:
- "`QueryView: !Send + !Sync` (inherits from `UnsafeEcsCell`)" is wrong twice
  over: `UnsafeEcsCell` IS `Send + Sync`, and `EcsMaster` IS `Send + Sync`.
- PHASE9.1 claim "Workers running concurrently NEVER see the cache" is
  factually wrong: workers DO have `&EcsMaster` access via `cell.world()`.
- The borrow checker DOES still gate `query<&mut self>`, but the asymmetric
  `query_ref<&self>` path becomes worker-reachable.

**What is needed**: rewrite §7 and QV1/QV7 to acknowledge
`EcsMaster: Send + Sync`. Explicitly state the actual gate: dispatcher
reborrows `&mut EcsMaster` outside `Schedule::run` per the apply-window
contract. Either prove `query_ref<&self>` cannot be reached through
`UnsafeEcsCell::world()`, or prove cache reads are safe.

### C2. `query_ref<&self>` cache-miss path is API-impossible: `QueryDataState::new` requires `&mut EcsMaster`.

**Where**: plan §4.3 line 489–499 + §5.1 line 805–809 + Q-B1.2 / QV6.

**Why critical**: `crates/boyko_ecs/src/ecs/core/iters/query/state.rs:69`
`pub fn new(world: &mut EcsMaster) -> Self`. The plan claims `query_ref<&self>`
builds a stack-allocated `QueryDataState` on cache miss — but
`QueryDataState::new` takes `&mut EcsMaster`. Even if the operation is
observationally `&self`-safe (it only reads `archetype_master()` and
`init_state` on `D`/`F`), the signature change to accept `&self` is a
load-bearing prerequisite that the plan does NOT call out as a separate Step.

**What is needed**: either drop `query_ref<&self>` (collapse onto `&mut self`-only
API for v1), or list `QueryDataState::new(&self EcsMaster)` signature
relaxation as an explicit Step in §9.

### C3. `Box<[OnceLock<NonNull<()>>; 1024]>` cannot be allocated on the heap with `Box::new(core::array::from_fn(|_| OnceLock::new()))` for a 1024-slot array of 24-B `OnceLock` because it overflows the stack before reaching the heap.

**Where**: plan §9.2 Step B1 — "Initialise in `new` and `with_capacity`".

The Phase 8.5 `bundle_archetype_cache` pattern works because `OnceLock<ArchetypeId>`
is `~12-16 B × 1024 = ~16 KB`. The new cache's slot is `(NonNull<()>, fn(NonNull<()>))`
— `OnceLock` size will likely be 24 B too. Stack pressure of `1024 × 24 = 24 KB`
may already be near the limit for typical thread default stacks.

The plan claims the cache is "16 KB" in §10.3 and "24 KB" in the same §10.3
recomputation — internally inconsistent.

**What is needed**:
(a) recompute the actual `size_of::<OnceLock<(NonNull<()>, fn(NonNull<()>))>>()`
    and pin the answer in an `assert_eq!` test (mirroring `oncelock_size_assumptions`
    at `bundle_type_registry.rs:286-310`).
(b) If the array temporary truly hits 16-24 KB on the stack, switch to
    `Box::<[OnceLock<...>; MAX_QUERY_TYPES]>::new_zeroed_with` (nightly) or a
    `Vec` of `OnceLock::new()` collected into a `Box<[OnceLock<...>]>` slice.
(c) Resolve §1.2 vs §10.3 inconsistency.

### C4. `SystemMeta::DUMMY` const constructibility — OQ-3 is a CRITICAL blocker disguised as an "open question".

**Where**: plan §4.4 (lines 504-525), §1.3 G, §15 OQ-3, §13 Risk row "Access::EMPTY const construction".

**Why critical**: Opt-B2's elision and Opt-B1's direct API both depend on
`SystemMeta::DUMMY` being `'static const`. The current code path:
`SystemMeta::DUMMY` needs `Access::EMPTY`, which needs `ComponentMask::new()`
to be `const fn`. Per `component_mask.rs:19` `ComponentMask::new()` is NOT
`const fn` (it calls `BitSet<u64>::new()` at `bit_set.rs:87` which is also
not `const`).

The plan's fallback (`OnceLock<SystemMeta>::DUMMY`) adds one Acquire load
to every direct `query` call — and would change the §1.2 cache-hit budget
from 3 ns to ~5 ns. More importantly, the dummy plumbing is the COMPILE GATE
for Opt-B2's elision (NCD5's default body forwards `set_table_readonly(_, _, _, &SystemMeta::DUMMY)`).

**What is needed**: Resolve BEFORE Wave A Step A1 starts. Concrete acceptance
criterion: write `const _: SystemMeta = SystemMeta::DUMMY;` at module scope
and confirm it compiles. If not, the plan must either
(a) `const fn`-ify `ComponentMask::new` / `BitSet::new` / `Access::new` (mechanical),
OR
(b) document the `OnceLock<SystemMeta>` fallback with updated §1.2 budget,
    AND fix Opt-B2's default-body to NOT use `DUMMY`.

Right now Opt-B2's NCD5 default body is *circular*: it forwards through `&DUMMY`,
but `DUMMY` doesn't const-construct.

### C5. Drop discipline of the cache via fn-pointer can run AFTER `arena` is dropped, leading to UB.

**Where**: plan §10.1, §13 row 7 "Drop ordering bug", §QC6.

Plan §10.1 places `query_state_cache` BEFORE `change_tick` / `last_check_tick` /
`arena`. Rust drops fields in declaration order, so `query_state_cache` drops
FIRST, then `arena`. Inside `query_state_cache::drop`, the per-slot fn-pointer
reconstructs `Box<QueryDataState<D, F>>` and runs its drop. The plan claims
"QueryDataState holds arena-independent storage" without citing the actual
layout audit. `D::State` and `F::State` are stored inline; the plan does
not exhaustively prove that all leaf `D::State` / `F::State` impls (especially
`Ref<T>` / `Mut<T>` Phase 10 fetches) hold no arena-derived raw pointers.

**What is needed**:
(a) add a doc-comment + `static_assertions::assert_eq_size` style invariant
    on every `D::State` / `F::State` impl that they hold no arena-derived
    raw pointers — OR drop `query_state_cache` AFTER `arena` (move it below
    the `arena: Box<Arena>` field).
(b) Add a Miri-tested regression: `miri_query_cache_drops_before_arena_with_arena_derived_d_state`.

### C6. The plan's primary success criterion is unmet by its own analysis.

**Where**: plan §1.2 row 2 — target ≤ 7.6 µs vs Bevy 6.90 µs (≥ 1.10× Bevy).

**Why critical**: Phase 12.5 umbrella success criterion (line 24):
`boyko ≥ 1.10× bevy`. For query iter the baseline is bevy 6.90 µs, so
boyko target = **6.27 µs or less**, not 7.6 µs. The plan's target 7.6 µs
is `boyko ≤ 1.10× bevy` (i.e. 10 % SLOWER cap), which is the OPPOSITE
direction of the umbrella's criterion.

Profile §2 also notes that the cached-system path is already at ~11 µs and
Bevy's direct API at ~9 µs — the plan's "remove ~2 µs of cell ferrying"
handwave does not show how the inner loop's already-at-parity codegen
reaches 6.27 µs when even the elimination of the entire system wrapper
leaves ~9 µs ≈ Bevy parity, not 1.10× faster.

**What is needed**: pick one:
(a) admit Track B closes the gap to parity but does not surpass Bevy by 10%,
    document the residual as Phase 13, and reword "boyko ≥ 1.10× bevy" to
    "boyko ≥ bevy" or "boyko at parity" for Track B;
OR
(b) identify the additional ~1 µs lever that takes the direct API from
    ~9 µs to 6.27 µs.

Do not ship the plan with an unattainable acceptance gate.

## IMPORTANT

### I1. `query_cold_init` Tree Borrows hygiene of `Box::leak` + `as_mut` projection chain.

§5.1 `state: &mut QueryDataState<D, F> = unsafe { typed.as_mut() }` produces
a `&mut` reborrow from a raw pointer, then immediately downgrades to `&*state`.
Tree Borrows may flag the `as_mut` as creating a `Unique` retag that
invalidates other live references derived from the same `NonNull`.

**Solution options**: (a) `Pin<Box<...>>` + `as_ref()` only. (b) `UnsafeCell<QueryDataState<...>>`.
(c) Add Miri test that calls `query::<D, F>()` 100 times in a row.

### I2. Process-global `QueryTypeId` cross-crate LTO pinning needed.

Phase 8.5 verified the pattern works for `BundleTypeId`; the QueryTypeId
pattern is identical EXCEPT the key is `(D, F)` (two-type tuple).

**Solution options**: (a) Cite Phase 8.5 LTO-pinned test. (b) Add regression
test `query_type_id_distinct_for_DistinctlyShaped_DF_pairs_under_lto`.
(c) Use the proven Phase 8.5 trait pattern verbatim.

### I3. `QueryViewRef` self-referential `Temporary(QueryDataState<D, F>)` variant cannot type-check.

`QueryViewRef::state` would borrow from `Self::Temporary(QueryDataState)` —
self-referential types are NOT expressible safely in safe Rust without `Pin`.

**Solution options**: (a) Drop `Temporary` variant entirely. (b) Return enum
at API level: `QueryRefResult { Cached(...), CacheMissed(BuilderHandle) }`.
(c) Own state by-value with `Pin<Box>` (176 B per miss).

### I4. `NEEDS_CHANGE_DETECTION` const NCD1 vs NCD5 default-body asymmetry.

NCD1 says "no default body" but NCD5's new methods `set_table_readonly_no_meta`
DO have default bodies forwarding through `&SystemMeta::DUMMY`. An impl
author who copies the existing `&T: QueryData` impl + adds the const +
forgets to override the no-meta method falls through to the default body
silently — "everything is changed" semantics.

**Solution options**: (a) Invert the default — `set_table_readonly_no_meta`'s
default panics unless `NEEDS_CHANGE_DETECTION = false`. (b) Remove the default
body entirely. (c) `const_assert!(!Self::NEEDS_CHANGE_DETECTION)` in default body.

### I5. `1024` MAX_QUERY_TYPES brittle without exposed knob.

The "fix" listed is "raise constant (one-line change)" but users do not own
`boyko_ecs/src/ecs/core/iters/query/query_type_registry.rs`.

**Solution options**: cargo feature `big_query_table`, or `RUSTFLAGS='--cfg max_query_types=N'`.

## OPTIONAL

### O1. §6.2 "300 ns cold cost" arithmetic breakdown is missing — verify component-by-component.

### O2. §11.3 regression budget 14.5 µs conflicts with umbrella "no regression" wording. Reconcile.

### O3. `OnceLock::set` `Err` arm in §5.1 should `panic!` not recurse (catches a real bug).

### O4. `QueryIter::new` is `pub(crate) unsafe fn` — pin same-crate access in §4 / §8.1.

## VERDICT

**NEEDS-FIX**

## RATIONALE

The plan has six critical defects: (C1) plan's concurrency story rests on
the false claim that `EcsMaster: !Send + !Sync`, while the code asserts
`Send + Sync` at compile time; (C2) `query_ref<&self>` path is API-impossible
because `QueryDataState::new` requires `&mut EcsMaster`; (C3) cache-footprint
math is internally inconsistent (16 KB vs 24 KB); (C4) `SystemMeta::DUMMY`
is not const-constructible today — load-bearing blocker for Opt-B2,
mis-classified as a critic-open-question; (C5) cache-slot fn-pointer drop
discipline lacks documented invariant on D::State / F::State; (C6) §1.2
target permits being 10% slower, opposite of umbrella criterion.

Five additional important issues (I1-I5) cover Tree Borrows hygiene of the
`Box::leak` + `as_mut` projection chain, cross-crate `QueryTypeId`
monomorphisation pinning, self-referential `QueryViewRef`, defaulted-no-meta
footgun, and `MAX_QUERY_TYPES` extensibility.

## Relevant file paths

- `D:\claude\BoykoEngine\docs\PHASE-12.5-QUERY-OPTIMIZATIONS-PLAN.md`
- `D:\claude\BoykoEngine\docs\PHASE-12.5-PROFILE-QUERY.md`
- `D:\claude\BoykoEngine\docs\PHASE-12.5-SURPASS-BEVY-PLAN.md` (umbrella — see C6)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\ecs_master\ecs_master.rs` (Send/Sync 1480-1510 — C1; field order 82-169 — C5)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\state.rs` (`QueryDataState::new` requires `&mut` — C2)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\query.rs`
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\iter.rs` (NCD6 injection at 232-247)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\par_iter.rs`
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\data.rs` (78-impl migration; tuple macro 1054)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\filter.rs`
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\system_meta.rs` (DUMMY const target — C4)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\access.rs` (`Access::new` not const — C4)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\component\component_mask.rs` (`ComponentMask::new` not const — C4)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\bundle\bundle_type_registry.rs` (template at 286-310)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\unsafe_ecs_cell.rs` (Send/Sync at 341-342 — C1)
