<!-- Beat-Bevy perf-gap closure plan. APPROVED by architecture-critic round 3 (0 CRITICAL). Implementation SEQUENCED AFTER the entity-bitset feature (shared spawn/query hot-path files). See docs/PERF-GAP-BEAT-BEVY.md for the measured baseline. -->

M2 truth-table values are now pinned from source:
- **`Commands`** (commands.rs:397) overrides `apply` → flushes the queue → **`HAS_DEFERRED = true`**.
- **`EventWriter`** (event_writer.rs:188-231) has **NO `apply` override** (only `init_state`/`init_access`/`get_param`); it writes into a heap-stable shared `EventBuffer` at `send`-time via per-lane TLS, no deferred flush → **`HAS_DEFERRED = false`**. (Confirms Phase-12 EW5 Option-A: events stay outside the deferred-apply machinery.)
- **`EventReader`/`Local`/`Res`/`ResMut`/`Query`** have no `apply` override → **`false`**.
- **`()` and tuples** → OR-fold of members.

C1 verified-and-resolved (Decision 3 dropped), M1/M2/M3/O1/O2/O3 inputs pinned. Producing the revised complete plan.

---

# Architecture: Beat Bevy on spawn_batch (g5) and SystemParam query iter (g2) — Revision 3

## Goal
Make boyko end **< Bevy 0.18.1** on the two losing benches, with a hard **0%-gate** on every winning path.
- **g5 spawn_batch 10k**: 222 µs → **< 177 µs** (need ≥ 1.26× self-speedup).
- **g2 SystemParam query iter 10k**: 7.86 µs → **< 6.70 µs** (need ≥ 1.17× self-speedup; direct-API g2b is already at parity, so the gap is entirely in the SystemParam dispatch envelope, of which the dominant fraction is a bench-harness artifact — see Decision 1).

## Context and constraints
- Subsystems touched: `boyko_macros` (Bundle derive), `spawn_batch_command`, `component_pool`/`component_pool_bundle`, `archetype`, `ecs_master` (`run_cached_system`), `function_system`, `system.rs` (System trait), `system_box.rs` + `schedule.rs` (executor), `system_param.rs` + all leaf params, `filtered_access_set`.
- Invariants preserved: B1 canonical-sorted column order; B4 panic→leak-never-double-drop; SBO* batch contracts; Phase-10 tick-stamping; Phase-14a/14b hook+observer fire semantics; Phase-9/9.1/9.2/9.3 executor soundness (Miri-TB clean); Phase-9 conflict-graph access surface; Phase-22.1 P1–P4 term-scratch reclaim-at-every-mint-funnel invariant (see Decision 3 = DROPPED).
- Target metrics: g5 < 17.7 ns/entity; g2 envelope overhead → near the bare-iter floor (g2b ≈ 6.73 µs).
- **PROFILE-FIRST is mandatory** (Phase-22.1 lesson). No fix ships on a guessed hotspot.

## Ground truth established by source audit (re-verified this round)
| Claim | Verified at | Status |
|---|---|---|
| `System` trait has `apply` w/ default empty body, **NO const** | `system.rs:56` (`unsafe trait System`), `:120` (`fn apply(&mut self,_world){}`) | CONFIRMED — const must be ADDED to `System` (C1-r1) |
| Production dispatcher is `Schedule::run`; systems stored type-erased | `schedule.rs:212`; apply `:686`/`:1043`, drain `:693`/`:1046`; `system_box.rs:74` = `Box<dyn System<Out=()>>` | CONFIRMED — const unreadable through `dyn`; needs cached runtime bool (C2-r1) |
| `SystemBox` already caches `is_exclusive` bool at build (precedent) | `system_box.rs:80,108` | CONFIRMED — `has_deferred` mirrors it |
| **Terms live in the VIEW, not the state; minted EMPTY per call** | `query.rs:75` (`Query.terms`), `:98-111` (`with_tag(mut self)` pushes at runtime), `:567` (`get_param` mints `TagTerms::EMPTY`), `query_view.rs:179` | CONFIRMED — **refutes Decision-3 premise (C1-r2)** |
| `reclaim_retired` must run at every mint funnel (P2) | `term_list.rs:39-62`, `:318-323` debug_assert, `:381-388` fast path = 1 Relaxed null-load + predicted branch | CONFIRMED — gating it `(D,F)`-static LEAKS the retired list (C1-r2) → **Decision 3 DROPPED** |
| `Commands::apply` overrides (flushes queue) | `commands.rs:397` | CONFIRMED — `Commands` HAS_DEFERRED = true (M2) |
| `EventWriter` has NO `apply` override (writes shared buffer at send-time) | `event_writer.rs:188-231` (only init/get_param), Phase-12 EW5 Option-A | CONFIRMED — `EventWriter` HAS_DEFERRED = **false** (M2) |
| SystemParam leaf set is exhaustive | grep: `Commands, EventWriter, EventReader, Local, Res, ResMut, Query, (), tuples`; no `State<S>`/`NextState<S>` leaf (they go through `Res`/`ResMut`) | CONFIRMED — closed leaf set (O3) |
| `run_cached_system` rebuilt per g2-iter; `initialize` heap-allocs 24 KB | `ecs_master.rs:1846,1877`; `function_system.rs:223`→`filtered_access_set.rs:139`; `new()` already `#[cold]` | CONFIRMED (O1) |
| Per-row `sort_unstable_by_key` + runtime-length memcpy + upfront `ManuallyDrop` | `boyko_macros/src/lib.rs:1316` (ManuallyDrop), `:1344` (sort), `component_pool.rs:1437` (`copy_nonoverlapping(.., layout.size())`) | CONFIRMED — spawn losers (M3/O1-r2) |

---

## Key decisions

### Decision 1 (P2): Fair-bench first — separate the bench-harness artifact from steady-state cost
**What**: Add `comparison_v2::g2c_boyko_query_iter_cached` that builds the `FunctionSystem` **once** outside `b.iter`, runs it once to warm `initialize`, then calls `run_cached_system(&mut sys)` in the loop (mirrors Bevy's pre-built `QueryState`). Profile true steady-state against g2c, not g2.
**Why**: g2 pays per-iter `into_system` + cold `initialize` (24 KB alloc/zero/free) every iteration; a real `Schedule` pays this once. Profiling against the wrong baseline is the 22.1 trap.
**Alternatives**: Optimize `FilteredAccessSet::new` as primary — rejected; it helps only the cold path a real schedule hits once (O1). Kept as cheap, optional, last (Decision 6).
**Trade-off**: Two g2 numbers to report. Honest; the roadmap already reports g2/g2b asymmetrically.

### Decision 2 (P2): runtime-cached `has_deferred` on the executor + `if const` on the monomorphic `run_cached_system`
Two-surface design (C1-r1 + C2-r1 resolutions):

**2a. Trait const (compile-time, monomorphic callers).** Add `const HAS_DEFERRED: bool` to the **`System` trait** (`system.rs:56`), default `true` (conservative — a custom `System` impl is assumed deferred until it opts out; `true` is safe-but-slow, never silent-data-loss). `FunctionSystem<F,M>` overrides: `const HAS_DEFERRED: bool = <F::Param as SystemParam>::HAS_DEFERRED;`. Add `const HAS_DEFERRED: bool` to **`SystemParam`** (`system_param.rs`), default **`false`**, tuple impl = OR-fold of members, leaf overrides per the M2-pinned truth table (below).

**2b. Monomorphic gate (`run_cached_system`, the g2c path).** `run_cached_system<S: System>` is generic ⇒ `S::HAS_DEFERRED` IS readable:
```rust
if const { S::HAS_DEFERRED } { system.apply(self); self.drain_deferred_hook_queue(); }
```
For a read-only `Query` system, `apply`+drain compile out → byte-identical dispatch tail to the direct API.

**2c. Runtime gate (the executor — `dyn System`).** The scheduler stores `Box<dyn System<Out=()>>`; an associated const is unreadable through a trait object. Add an object-safe `fn has_deferred(&self) -> bool { Self::HAS_DEFERRED }` to `System`, cache it in `SystemBox` as a build-time `bool` (exactly like `is_exclusive`, `system_box.rs:80/108`). Gate **both** executor apply sites:
- serial `schedule.rs:1043` → `if self.systems[i].has_deferred { ...apply...; drain }`
- parallel apply-window `schedule.rs:686` → same.

**Why**: 2a/2b give the monomorphic micro-path (g2c) zero-cost elision; 2c is the only way to reach the production dispatcher through `dyn`, reusing the proven `SystemBox` runtime-bool precedent. A read-only system in a real `Schedule` skips the `apply` virtual call + `DeferredScopeGuard::enter()`/drain entirely.
**Alternatives**: const-only (rejected — unreadable through `dyn`); runtime-bool everywhere incl. `run_cached_system` (rejected — leaves a branch where `if const` removes it). Hybrid is correct.
**Trade-off**: one const through `System`+`SystemParam`+tuple macro+leaves, one cached bool in `SystemBox`, two executor branches. The C2 executor 0%-gate (Wave 3, M1-r2 below) is mandatory and binding-by-asm.

**M2-pinned `HAS_DEFERRED` truth table (verified from source this round — encoded by the leaf-audit test, NOT discovered at codegen):**
| Param | `HAS_DEFERRED` | Source evidence |
|---|---|---|
| `Commands` | **true** | `commands.rs:397` overrides `apply` → `state.apply(world)` |
| `EventWriter` | **false** | `event_writer.rs:188-231` no `apply`; writes shared buffer at send-time (EW5 Option-A) |
| `EventReader` | **false** | `event_reader.rs:312` no `apply` |
| `Local` | **false** | `local.rs:98` no `apply` |
| `Res` | **false** | `res.rs:85` no `apply` |
| `ResMut` | **false** | `resmut.rs:80` no `apply` |
| `Query` | **false** | `query.rs:482` no `apply` |
| `()` | **false** | `tuple_impl.rs:54` no `apply` |
| `(P0,…,Pn)` | OR-fold | `tuple_impl.rs:90/196` — `P0::HAS_DEFERRED \|\| … \|\| Pn::HAS_DEFERRED` |

### Decision 3 (P2): ~~Gate `reclaim_retired()` behind a "terms-ever-used" flag~~ — **DROPPED (C1-r2 BLOCKER, verified)**
**Status: REMOVED.** Source refutes the premise. Terms live in the **view** (`Query.terms: TagTerms`, query.rs:75), are minted EMPTY per `get_param` (query.rs:567), and `.with_tag()`/`.without_tag()` push at **runtime** via `mut self` (query.rs:98-111). The **same** `QueryDataState` slot (keyed by `(D,F)` via `query_type_id`) backs both a term-free view and a `.with_tag()` view on different calls — so `(D,F)` does NOT determine whether terms are ever used; the caller decides per-call. `reclaim_retired` at the mint funnel (query.rs:557) exists precisely because a *prior* call's `.with_tag()` could have published+retired a `TermList` into that slot's `TermScratch` (P2, term_list.rs:39-62; debug_assert `:318-323` "reclaim_retired must run at every mint funnel before the next retire"). A `(D,F)`-static `has_terms==false` gate would, for any slot used term-free this frame but term-bearing earlier, **LEAK the retired `TermList` forever** and break the P2 reclaim-at-every-funnel invariant (release: leak; with a later retire on the same slot, a double-pending → P2 violation the debug_assert only catches in debug). Moreover the achievable saving is near-zero: `reclaim_retired`'s fast path is already a single `Relaxed` null-load + predicted-not-taken branch (term_list.rs:381-388), and research itself rates this "not the dominant gap." There is no sound, beneficial redesign at the `QueryDataState` level (it has no `term_count`; terms are per-view). **Dropped entirely.** The P2 invariant is therefore left untouched — `reclaim_retired` keeps running at every mint funnel on both the direct API and the SystemParam path (so g2b stays byte-identical regardless).
**Net effect on the goal**: g2's win now rests entirely on Decision 1 (honest framing) + Decision 2 (executor/monomorphic apply-drain elision). This is sufficient: g2b (direct API) is already at parity with Bevy, and Decision 2c removes the SystemParam-only `apply`+drain envelope from `run_cached_system` (g2c) and the production executor — closing the gap without touching the soundness-critical term path.

### Decision 4 (P1): Kill the per-row sort + runtime memcpy — emit a monomorphized, pre-sorted, fixed-width typed batch write (TB + B4 contracts pinned per M3, O1-r2)
**What**: Add a derive-emitted `Bundle` method:
```rust
// `BundleColumnPtrs` is a STACK-LOCAL built once per batch under the SAME
// `&mut Archetype` reborrow that owns the pools; per data column it holds a
// raw write base pointer + const-known stride. The row loop writes through
// THOSE pointers only and never re-derefs the archetype.
unsafe fn write_row_typed(self, dst: &BundleColumnPtrs, row: usize);
```
Field→column permutation built **once per batch** (not per row): the macro emits fields in declaration order; the per-batch code maps each declaration field to its canonical pool slot (the canonical sorted order already in `BundleColumnCache`). The per-row body does, for each field `k` (const-unrolled over arity): `ptr::write::<Tk>(dst.col[perm[k]].base.add(row * STRIDE_k), self.field_k)` where `STRIDE_k = size_of::<Tk>()` is a compile-time constant ⇒ **fixed-width store** (Bevy's `OwningPtr` move advantage), no `sort_unstable_by_key`, no runtime-length memcpy.

**Drop-suppression discipline (O1-r2 resolution — explicit, mirrors the existing macro):** `write_row_typed` consumes `self` by value; each field is moved out via the existing upfront-`ManuallyDrop` discipline (`boyko_macros/src/lib.rs:1316`) — wrap `self` so fields are `ManuallyDrop<Tk>`, then `ptr::write(dst, ManuallyDrop::take(&mut field_k))` (or `core::ptr::read` of the `ManuallyDrop` field then `ptr::write` to the column). Each field is moved exactly once into its column slot and is NOT dropped at the end of `write_row_typed` (it was logically relocated). This guarantees: if `iter.next()` panics **between** rows, no already-relocated field of a *completed* row is double-dropped (the source bundle for that row no longer owns them), and the partially-written current row never had `write_row_typed` called.

**TB / aliasing contract (M3 resolution — explicit, the 14a-F2/9.3c/Ph19 antidote):**
1. `BundleColumnPtrs` base pointers resolved **once, freshly**, under the Step-3 `&mut Archetype` reborrow (the existing `component_pools_mut()` borrow in the batch command). They are **stack-local raw `*mut u8`** for the row loop only — never stored in a struct that outlives the borrow, never cached across a re-borrow of `component_pools_mut()`.
2. The row loop writes **exclusively through those resolved pointers**. It does **NOT** call `archetype.component_pools_mut()` (or any pool accessor) again inside the loop. Exactly one live `&mut`-derived provenance chain (resolved up front); no second access path re-tags the pointee mid-loop.
3. **No cached `NonNull` across a reborrow**: `BundleColumnPtrs` lives entirely within one function frame; constructed and consumed without any intervening `&mut EcsMaster`/`&mut Archetype` reborrow.

**B4 partial-panic drop contract (M3 resolution — re-proven, with a Drop-during-move corner covered per O1-r2):**
- `write_row_typed` relocates each field via `ptr::write` (drop suppressed for written fields — slot logically initialized exactly once, no double-drop).
- Panic from `iter.next()` **before** `write_row_typed` for row `i`: rows `0..i` written; row `i` not written; `commit_units_batch` is **NOT** called on the panicking row (commit happens **after** the whole write loop, `spawn_batch_command.rs:465`), so the pool `len` is **not** advanced to expose any half-written/unwritten slot → no leak, no UB, identical to the current contract.
- **Drop-during-move corner (O1-r2):** a field whose own `Drop` runs during the move-out cannot occur — `ptr::write`/`ManuallyDrop::take` perform a bitwise relocation and do not run `Drop`. The ONLY way a field's `Drop` runs mid-`write_row_typed` is if a *later* field's relocation panicked, but `ptr::write`/`read` cannot panic (no user code). So `write_row_typed` is panic-free internally; the sole panic source remains `iter.next()` between rows. The M3 drop-count-exact test nonetheless explicitly covers "a bundle field with a panicking `Drop`" to prove the typed path never invokes field `Drop` during the move (it relocates, suppressing source `Drop`).

**Why**: eliminates both the per-row sort (n redundant sorts) and the dynamic memcpy → fixed-width SIMD-friendly stores. Single most-likely sub-Bevy lever per research + audit.
**Alternatives**: hoist the sort out but keep variable-length memcpy — rejected; still no fixed-width stores. Typed write gets both wins.
**Trade-off**: new derive method + per-batch permutation build + generated code per Bundle type (I-cache cost — bounded per O2, Decision 4b). `for_each_data_component_bytes`/`for_each_component_bytes` **retained** for single-spawn + migration callers (no deletion → no g4/insert churn).

### Decision 4b (P1, O2 resolution): bound the typed-write I-cache cost; high-arity fallback
**What**: Wave-0 Step 0.3 measures release codegen size of `write_row_typed` for representative high-arity bundles (8 and 16 fields) vs the retained `for_each_data_component_bytes` path. If the const-unrolled body inflates the spawn hot path's I-cache footprint beyond the byte path, keep a compile-time arity threshold (`const MAX_TYPED_WRITE_ARITY`): bundles with arity ≤ threshold use `write_row_typed`; above it, fall back to `for_each_data_component_bytes` (retained anyway). Common spawn workload is low-arity (1–4 fields) where unrolling is a pure win.
**Why**: bounds the I-cache acknowledgment instead of leaving it open.
**Trade-off**: one `const` threshold + a `cfg`/generic branch in the derive emission. Decided by measurement.

### Decision 5 (P1, profiling-gated): collapse `reserve_capacity` to a single pool pass
**What**: replace the two-pass (Phase A `can_reserve`, Phase B `grow_rows`, `archetype.rs:824`) with a single walk that precomputes the per-pool target arithmetically then grows in one traversal; fail before mutating if any pool can't reserve.
**Why**: fewer loop-setup costs. **Secondary** — at 2 pools cheap; 22.1 says the real floor was VM-commit. Ship only if Step-0.1 attribution shows pool-iteration cost is material.
**Trade-off**: minor structural simplification.

### Decision 6 (P1, profiling-gated): VM-commit / growth attribution + pre-commit (with its OWN 0%-gate per O2-r2)
**What**: add a warm-world spawn_batch sub-bench (cross the commit frontier once, then measure repeated spawn past the frontier) vs fresh-world g5. If the fresh-world delta is dominated by the first `vm.commit` syscall (`component_pool.rs:365`), the candidate fix is **pre-commit on archetype creation** for an expected batch size (size heuristic, RSS trade — Open Q3), or accept it (Bevy pays growth too). Decision deferred to profiling output.
**O2-r2 guard (mandatory if D6 ships):** pre-commit-on-create trades RSS for latency on **every** archetype create, including archetypes that never receive a large batch, and the bench rebuilds the world per iter — so D6 risks regressing the Phase-X.C-optimized `EcsMaster::new` (~7.23 µs) and small-world create paths. **D6 carries its OWN 0%-gate benches**: `arena_new`/`ecs_master_new` (Phase-X.C) and a small-world (1-entity, 1-archetype) create-and-spawn micro MUST be A/B clean (≤ flat) for D6 to ship. If pre-commit regresses either, scope D6 to a per-batch amortization that only commits when a batch actually crosses the frontier (no eager create-time commit), or drop D6 and accept the growth cost (Bevy pays it too).
**Why**: 22.1 fingered VM-commit as the spawn floor. Measure before optimizing the row loop.

### Decision 7 (P2, O1 resolution — explicitly scoped): optional cold-init cheapening
**What**: optionally make `FilteredAccessSet`'s 24 KB `Box` stack-resident / pooled / lazily-allocated.
**Why & scope (O1)**: `filtered_access_set.rs:new()` is **already `#[cold]`**, the 24 KB Box is **already** documented transient/freed by `finalize`, reached only on the **first** `initialize` (FS1 short-circuits `state.is_some()`, `function_system.rs:188`). On the cached g2c path and any real `Schedule` it runs **exactly once**. Helps **only g2-bench noise / a late-added system, never steady-state**. Deferred, optional, last (Wave 1 Step 1.5), only if Wave-0 shows cold-init noise materially obscures the g2/g2c attribution. **Not** a primary win lever.

---

## Algorithms for critical paths

### Spawn_batch per-row write (post-fix, Decision 4)
```
per batch (once, under the single &mut Archetype reborrow):
  resolve archetype + canonical pool_ids                         // existing cache
  build BundleColumnPtrs (stack-local): per data column -> (base *mut u8, const stride)
  build perm[k]: decl-field-index -> data-column slot            // once, not per row
per row i in 0..n:
  bundle = iter.next()                                           // only panic source
  // self wrapped so fields are ManuallyDrop<Tk> (drop-suppression discipline)
  for k in 0..ARITY (const-unrolled, ZSTs const-folded out):
    ptr::write::<Tk>(BundleColumnPtrs.col[perm[k]].base.add((start+i)*STRIDE_k),
                     ManuallyDrop::take(&mut self.field_k))
// commit_units_batch + fill_ticks_batch AFTER the loop (unchanged) — B4 guard intact
```
- Complexity: O(n·arity), arity const-unrolled → straight-line, no per-row branch.
- Cache: streaming sequential writes per column. NT stores deferred to measurement (Open Q2 / O3-r1).
- Branching: none in the row body.
- SIMD: fixed-width `ptr::write::<Tk>`; outer row loop vectorizable for single-field POD bundles (Bevy parity).

### g2 SystemParam dispatch — monomorphic tail (run_cached_system, post-Decision 2b)
```
run_cached_system::<S>(sys):
  sys.initialize(self)                  // FS1 short-circuits warm
  cell = new_mutable(self)
  out = sys.run_unsafe(cell)            // get_param (reclaim_retired UNCHANGED — D3 dropped) + body
  if const { S::HAS_DEFERRED } { sys.apply(self); self.drain_deferred_hook_queue() }
  out                                   // read-only Query: apply+drain compiled OUT
```

### Executor dispatch — type-erased tail (Schedule::run, post-Decision 2c)
```
serial (schedule.rs:1032-1046) / apply-window (schedule.rs:686-693):
  ...run_unsafe...
  if self.systems[i].has_deferred {     // runtime bool cached in SystemBox (like is_exclusive)
      scope = DeferredScopeGuard::enter();
      systems[i].system.apply(world);   // virtual call SKIPPED for read-only systems
      drop(scope);
      world.drain_deferred_hook_queue();
  }
```
- For a deferred system (`has_deferred == true`): the arm body MUST lower byte-identical to pre-fix code (only delta: a predicted-taken branch) → C2 0%-gate (M1-r2).
- For a read-only system: skips a virtual `apply` + guard + drain → strict improvement.

## Multithreading model
- No new shared state. Spawn-batch apply runs under exclusive `&mut EcsMaster` (SCH7). g2c dispatch single-threaded.
- `HAS_DEFERRED` (const) and `SystemBox.has_deferred` (build-time bool, immutable after build) introduce no runtime sync. The executor branch reads an immutable field — no atomics, no ordering; it sits inside the existing apply-window where `running == 0` (no live worker), so it cannot race a worker (Phase-9 apply-window invariant, schedule.rs:550-562).
- Decision 3 dropped ⇒ the term-scratch P1–P4 protocol is untouched; no new loom obligation, and the existing reclaim-at-every-mint-funnel invariant is preserved.
- `Send`/`Sync` unchanged. No new `unsafe impl`.

## Unsafe delta
- **Decision 4** adds generated `unsafe fn write_row_typed`. SAFETY: `dst` pointers carry write-capable provenance from the archetype pools resolved under one `&mut`; `row < committed_rows` (pre-reserved before the loop); each `ptr::write::<Tk>` targets a disjoint, uninit, aligned slot; field relocated via `ManuallyDrop::take` (source drop suppressed; written once; no `Drop` runs during the move — `ptr::write`/`read` are panic-free). **Net unsafe**: replaces the existing per-row `slice::from_raw_parts` + `copy_nonoverlapping` pair with `ptr::write` — comparable-or-smaller surface (no raw slice synthesis per row). Old byte path retained (no deletion).
- Decisions 2/5/6/7 add **no** new unsafe. Decision 2c reads an existing-pattern `bool` field; the `apply`/drain it gates is unchanged code. Decision 3 dropped ⇒ no change to the term-scratch unsafe surface.

## Integration
- `system.rs`: add `const HAS_DEFERRED: bool = true;` + `fn has_deferred(&self) -> bool { Self::HAS_DEFERRED }` (object-safe; default reads the const).
- `function_system.rs`: `FunctionSystem` overrides `const HAS_DEFERRED = <F::Param as SystemParam>::HAS_DEFERRED;`.
- `system_param.rs` + leaves: add `const HAS_DEFERRED: bool = false;` to `SystemParam`; `commands.rs` overrides `= true`; tuple impl OR-folds; all other leaves inherit `false` (M2 table).
- `system_box.rs`: add `has_deferred: bool`; `SystemBox::new` caches `system.has_deferred()` (alongside `is_exclusive`).
- `schedule.rs`: two `if has_deferred` gates (serial :1043, apply-window :686).
- `ecs_master.rs`: `run_cached_system` `if const { S::HAS_DEFERRED }` tail gate.
- `bundle.rs` + `boyko_macros`: `write_row_typed` + `BundleColumnPtrs` + per-batch perm + ManuallyDrop discipline; `spawn_batch_command.rs` row-loop swap (Decision 4b arity fallback).
- **query.rs/state.rs/term_list.rs: UNCHANGED** (Decision 3 dropped).
- Optional/profiling-gated: `archetype.rs` single-pass reserve (D5); VM pre-commit (D6); `filtered_access_set` cheapening (D7).

## Implementation plan

> **Every dev-wave brief MUST begin with the graphify-first rule**: *"Run `graphify query/explain/path` to orient before reading source; read raw files only after graphify, or to edit/debug specific lines. This applies to you and any sub-task."* Exploration briefs (enumerating SystemParam leaves, finding apply/fire sites) MUST start with `graphify query`/`explain` to avoid the Phase-14b undercount class (O3-r2).

### Wave 0 — PROFILING & ATTRIBUTION (gates everything; nothing ships before this)
- **Step 0.1 (P1 attribution)**: criterion sub-benches in `bench_bevy_vs_boyko` isolating, on fresh-world g5: (a) entity-id reservation (`reserve_batch`+`register_batch`), (b) `reserve_capacity`/`grow_rows` incl. VM-commit, (c) the per-row write loop, (d) `commit_units_batch`+`fill_ticks_batch`. Add a warm-world variant (D6). Capture release asm of the row loop (confirm sort + dynamic memcpy; confirm whether `fill_ticks` vectorizes — 22.1 says yes, so deprioritize touching it).
  - **Decision tree**: write-loop dominates → ship D4. VM-commit dominates → ship D6 (subject to its O2-r2 own-gate), D4 still helps steady-state. reserve two-pass material → ship D5. fill_ticks already vectorized → do not touch.
- **Step 0.2 (P2 attribution)**: add `g2c` cached sub-bench (D1). Profile g2c vs g2b. If g2c ≈ g2b → the 17% is per-iter cold init (24 KB) → fix is honest g2c reporting + optional D7. If g2c > g2b → trim the warm tail (D2b monomorphic gate) until g2c ≤ g2b. Capture asm of both the monomorphic tail (run_cached_system) and the executor tail.
- **Step 0.3 (O2 attribution)**: measure `write_row_typed` codegen size for 1/4/8/16-field bundles vs the byte path; set `MAX_TYPED_WRITE_ARITY` if the high-arity body bloats the spawn I-cache (D4b).
- **Step 0.4 (M2 confirmation deliverable, O3 — done this round, re-confirmed in-tree before Wave 1)**: read every leaf's `apply` and encode the truth table. **Already resolved from source**: `Commands=true` (commands.rs:397), `EventWriter=false` (event_writer.rs:188-231, no apply), `EventReader/Local/Res/ResMut/Query=false`, `()`=false, tuples=OR-fold. Wave-1 codegen ENCODES these values; the leaf-audit test asserts them — discovery does not happen at codegen time.

### Wave 1 — P2 envelope (only the fixes Wave-0 justifies)
- **Step 1.1**: `SystemParam::HAS_DEFERRED` const (default false) + `Commands` override `true` + tuple OR-fold + the M1-r1 mandatory leaf-audit test (below, encoding the Step-0.4 truth table).
- **Step 1.2**: `System::HAS_DEFERRED` const (default true) + object-safe `has_deferred()`; `FunctionSystem` override forwarding the param const.
- **Step 1.3**: `SystemBox.has_deferred` field + cache in `new`; **executor gates** at `schedule.rs:686` and `:1043` (C2 — touches the executor hot path; C2 0%-gate is Wave-3 mandatory, M1-r2).
- **Step 1.4**: `run_cached_system` `if const { S::HAS_DEFERRED }` monomorphic tail gate.
- ~~Step 1.5 (D3)~~ — **REMOVED (Decision 3 dropped).**
- **Step 1.5** (optional, only if Wave-0 shows cold-init noise material): `FilteredAccessSet` cheapening (D7 — cold-path-only, never steady-state).

### Wave 2 — P1 spawn (only the fixes Wave-0 justifies)
- **Step 2.1**: derive `write_row_typed` + `BundleColumnPtrs` (TB contract per M3, ManuallyDrop discipline per O1-r2) + per-batch perm + arity fallback (D4b); row-loop swap at `spawn_batch_command.rs:422-463`. Retain the byte path.
- **Step 2.2** (profiling-gated): `reserve_capacity` single-pass (D5).
- **Step 2.3** (profiling-gated): VM pre-commit / amortization (D6) — subject to its own O2-r2 0%-gate.

### Wave 3 — Verification & 0%-gate A/B
- C2 executor 0%-gate (mandatory, M1-r2 below); D6 own-gate (O2-r2) if D6 shipped; all win-target + 0%-gate benches; Miri-TB; tests.

## Metrics and validation

### Win-target benches (must end < Bevy)
- `comparison_v2::g5_*spawn_batch*` (10k): boyko < 177 µs.
- `comparison::g2_boyko_query_iter_10k` < `g2_bevy_query_iter_10k` (6.70 µs); **and** new `g2c` cached ≤ g2b (direct, 6.73 µs).

### 0%-gate benches (A/B clean vs git-stash pre-fix AND within-run vs Bevy — must NOT regress)
- `comparison::g1` (scheduler 50-sys, 1.61×), `comparison::g3` (par_iter, 3.42×), `comparison::g4` (single spawn ×10k, 1.21×), `comparison_v2::g2b` (direct-API query, parity).
- boyko-internal: `phase9_scheduler`, `phase12_5_spawn_batch`, `query_iter`, `query_dsl`, plus `bundle_static_cache`, `random_access` (touched-file proximity).
- **D6-specific 0%-gate (O2-r2, only if D6 ships)**: `arena_new`/`ecs_master_new` (Phase-X.C ~7.23 µs) + a small-world (1-entity/1-archetype) create-and-spawn micro — ≤ flat or D6 is rescoped/dropped.
- **g2b byte-identical**: direct API does not go through `run_cached_system` or the executor; Decision 3 dropped ⇒ `reclaim_retired` unchanged on the direct path → assert g2b asm unchanged.

### C2 EXECUTOR 0%-GATE (mandatory, falsifiable — M1-r2 resolution)
The risky surface is the `has_deferred == true` arm of the executor (the existing deferred dispatch behind a new branch). `phase9_scheduler` uses deferred-FREE closures, so it exercises ONLY the `false` arm and CANNOT validate the true-arm. Therefore the gate is:
1. **PRIMARY true-arm gate = a new deferred-heavy scheduler micro** (50 `Commands` systems, serial + parallel apply-window). This is the bench that exercises the risky arm.
2. **Binding evidence = asm A/B (not criterion)**: the `has_deferred == true` arm MUST lower **byte-identical-modulo-one-predicted-branch** to the pre-fix `apply`+guard+drain code. This is the authoritative proof, because a single predicted branch is below criterion's noise floor and criterion noise would swamp it.
3. **Falsifiable regression criterion**: "measurable regression" = **p < 0.05 over N ≥ 10 paired runs** (git-stash A/B), NOT a single run. g4-class ±20-30% noise (per 12.6) means single-run deltas are meaningless; the multi-run paired test is the only valid signal.
4. **`false`-arm gate = `phase9_scheduler`** must be ≥ flat (it removes work — read-only systems skip apply/drain; it can only improve or stay flat).
5. **Revert/rescope fallback**: if (2) shows the true-arm does NOT lower byte-identical-modulo-one-branch, OR (3) shows a true-arm regression at p<0.05, the executor gate (2c) is reverted and the win is scoped to `run_cached_system` only (Decision 2b), with a documented note that real-`Schedule` read-only systems keep the `apply`/drain overhead. Expectation is strict-improvement-or-flat; this is the documented escape hatch.

### Tests (mandatory)
- **M1-r1 leaf-audit test (mandatory, silent-data-loss guard)**: for EVERY `SystemParam` leaf assert `HAS_DEFERRED ==` the M2-pinned value (table above). The test ENCODES the source-verified truth table (`Commands=true`, all others `false`, tuples OR-fold) — it asserts, it does not discover. Plus a behavioral test: a system whose param set is `HAS_DEFERRED == true` (e.g. `Commands`) still applies through **both** `run_cached_system` AND `Schedule::run` (serial + parallel apply-window) — directly guarding the Phase-8 Step-10 silent-no-op class; and a behavioral test that an `EventWriter`-only system (`HAS_DEFERRED == false`) still delivers its events (proving the `false` classification does not drop EventWriter's send-time writes, since EventWriter has no `apply` to skip).
- **M3 spawn tests**: `write_row_typed` golden-bytes equality vs the byte path over 1-/2-/mixed-ZST-/16-field bundles; ZST tag committed + tick-stamped (`Added<Tag>`); B4 partial-iter-panic drop-count-exact on the typed path (panic from `iter.next()` mid-batch leaves rows committed-exactly, no half-row exposed); **O1-r2 corner: a bundle field with a panicking `Drop` — assert the typed path never invokes the field's `Drop` during the move-out (relocation suppresses source Drop)**; property test spawn N∈{0,1,2,8191} read-back-equal.
- **Miri-TB**: spawn_batch typed-write path (M3 contract — `BundleColumnPtrs` stack-local, no cached `NonNull` foreign-write, single provenance chain) + executor gate dispatch. Recall the 14a/19/9.3c TB-UAF class; the perm/ptrs array is stack-local, no reborrow inside the loop.
- **No new term-scratch tests** (Decision 3 dropped; existing loom/Miri P1–P4 gates remain the authority and are untouched).
- **debug_assert!**: perm-length == data-column count; `row < committed_rows`; `size_of::<Tk>() == pool layout size` per field; tuple `HAS_DEFERRED` OR-fold correctness.

### A/B methodology (hard, per roadmap)
Same-machine, back-to-back, drift-robust within-run ratio vs Bevy AND git-stash A/B vs pre-fix. Idle machine. Multi-run (N ≥ 10 paired) for noisy benches (g4 ±20-30% per 12.6) and the C2 true-arm gate.

## P4 — Sequencing & merge discipline (overlap with entity-bitset)
Per roadmap §Status this work is **blocked on entity-bitset landing first** — do NOT interleave.
- **Overlapping files**: `spawn_batch_command.rs` (tag columns in the batch path — impacts Decision 4), `boyko_macros/src/lib.rs` (Bundle derive tag emission), `component_pool*` (tag pool storage), `tag_terms.rs`/`term_list.rs` (uncommitted, entity-bitset-owned), and `query.rs`/`iter.rs`/`state.rs` only as merge-adjacency (Decision 3 dropped ⇒ this plan no longer edits the query term path, removing the prior round's biggest overlap risk).
- **Discipline**: (1) land entity-bitset fully (build green + its tests). (2) Rebase this work onto it. (3) **Re-run Wave-0 profiling after the rebase** — entity-bitset may shift the hot path. (4) Decision 4's `write_row_typed` must skip ZST/tag columns exactly as `for_each_data_component_bytes` does (tags committed+stamped post-loop). (5) Because Decision 3 is dropped, there is no longer a `has_terms` premise to re-validate against entity-bitset's dynamic-term changes — the term-scratch path stays as entity-bitset leaves it.

## Open questions
1. **(g2 framing)** Does a real `Schedule` already amortize `into_system`/cold-init to once? Yes — `phase9_scheduler` inits once. So g2's 17% is largely a bench artifact; the honest steady-state deliverable is g2c, and the *production* win comes from Decision 2c's executor gate (read-only systems skip apply/drain in `Schedule::run`). Report both g2 and g2c. Resolved by Wave-0 Step 0.2.
2. **(NT stores)** Can `write_row_typed` use non-temporal stores for large bundles? Default to plain `ptr::write`. **O3-r1 caveat (accepted):** the benefit threshold must be measured as the **full spawn + first-query** cost, not the spawn loop in isolation — spawned rows are typically read next frame, so NT stores can regress the frame even while looking good in the spawn micro. Only adopt if the combined metric improves.
3. **(VM pre-commit)** Is per-batch VM-commit amortizable without over-committing on small worlds? Pre-commit-on-create trades RSS for latency; needs a size heuristic; only if Wave-0 shows commit dominates AND it passes its own O2-r2 0%-gate (D6).

## Changelog (Revision 3 — addressing the round-2 critique)
- **C1 (CRITICAL/BLOCKER, fixed by DROPPING Decision 3)**: Verified against source — terms live in the **view** (`query.rs:75` `Query.terms`), are minted EMPTY per `get_param` (`:567`), and `.with_tag()`/`.without_tag()` push at runtime via `mut self` (`:98-111`) on the **same `(D,F)`-keyed `QueryDataState` slot**. So `(D,F)` does NOT determine whether terms are used, and `reclaim_retired` MUST run at every mint funnel (P2, term_list.rs:39-62/318-323) to free a *prior* call's retired `TermList`. A `(D,F)`-static `has_terms==false` gate would leak the retired list and break P2 (the 22.1 wrong-culprit class). The saving was already near-zero (fast path = 1 Relaxed null-load + predicted branch, term_list.rs:381-388) and research rated it "not the dominant gap." **Decision 3 is dropped entirely** (option (a) of the critic's requirement); the term path is left byte-identical, so g2b cannot regress and g2's win rests on Decisions 1 + 2. The non-sensical `has_terms == (term_count > 0)` debug_assert (QueryDataState has no `term_count`) is removed with the decision.
- **M1 (MAJOR, fixed)**: The C2 executor 0%-gate is now falsifiable: (i) the **deferred-heavy 50-`Commands` micro** is named the PRIMARY true-arm gate (phase9_scheduler is deferred-FREE and validates only the false-arm); (ii) "measurable regression" = **p<0.05 over N≥10 paired git-stash runs**, not single-run (g4 ±20-30% noise per 12.6); (iii) **asm A/B (true-arm byte-identical-modulo-one-branch) is the binding evidence** because a single predicted branch is below criterion's noise floor. Explicit revert/rescope-to-`run_cached_system` fallback retained.
- **M2 (MAJOR, fixed)**: `EventWriter::apply` resolved from source as a **Wave-0 deliverable, not a Wave-1 discovery**: `event_writer.rs:188-231` has NO `apply` override (writes the shared `EventBuffer` at send-time per EW5 Option-A) ⇒ `EventWriter HAS_DEFERRED = false`. The full truth table is pinned from source (Commands=true via commands.rs:397; all others false) and the leaf-audit test now ENCODES the verified values. Added a behavioral test that an EventWriter-only system still delivers events (proving `false` does not drop its send-time writes).
- **O1 (MINOR, addressed)**: Decision 4's drop-suppression discipline is pinned explicitly: `write_row_typed` consumes `self` by value with the existing upfront-`ManuallyDrop` pattern (lib.rs:1316), relocates each field via `ptr::write`/`ManuallyDrop::take` (suppressing source `Drop`), and the SAFETY comment states it. The M3 drop-count-exact test now covers a **bundle field with a panicking `Drop`**, proving the typed path never invokes field `Drop` during the move-out.
- **O2 (MINOR, addressed)**: Decision 6 (VM pre-commit) now carries its **OWN 0%-gate** — `arena_new`/`ecs_master_new` (Phase-X.C ~7.23 µs) + a small-world create-and-spawn micro must be ≤ flat or D6 is rescoped (commit only on actual frontier-crossing, no eager create-time commit) or dropped.
- **O3 (MINOR, addressed)**: The SystemParam leaf set is now exhaustively enumerated via grep (`Commands, EventWriter, EventReader, Local, Res, ResMut, Query, (), tuples`; no `State<S>`/`NextState<S>` leaf — they route through `Res`/`ResMut`), and exploration dev-briefs are mandated to start with `graphify query`/`explain` to avoid the Phase-14b undercount class. A missed leaf inherits the safe default (`SystemParam` default `false` would be the only data-loss risk, but the closed enumerated set + the M1-r1 leaf-audit test that asserts every leaf's value closes this).

### Rejected remarks
None — all six round-2 remarks (1 CRITICAL/BLOCKER, 2 MAJOR, 3 MINOR) accepted and addressed. C1 was verified against source (term_list.rs + query.rs) and resolved by dropping Decision 3, exactly as the critic's option (a) required.

**Key files**: `D:\claude\BoykoEngine\crates\boyko_macros\src\lib.rs` (1297-1359, ManuallyDrop at 1316, sort at 1344), `...\spawn_batch_command.rs` (422-471), `...\component_pool.rs` (1410-1521), `...\archetype.rs` (824-854), `...\ecs_master.rs` (1841-1899), `...\function_system.rs` (183-280), `...\system\system.rs` (56-120), `...\system\system_param.rs`, `...\system\params\commands.rs` (345-400), `...\system\params\event_writer.rs` (188-231), `...\system\params\{event_reader,local,res,resmut}.rs`, `...\system\params\tuple_impl.rs` (54,90,196), `...\schedule\system_box.rs` (71-115), `...\schedule\schedule.rs` (641-693, 1032-1046), `...\filtered_access_set.rs` (121-141), `...\iters\query\query.rs` (75,98-111,482,557,567 — UNCHANGED), `...\iters\query\term_list.rs` (UNCHANGED), `...\bench_bevy_vs_boyko\benches\comparison.rs` / `comparison_v2.rs`.

---

## Critic verdict

APPROVED (round 3). Convergence: R1 REVISE(2C/3M/3m) -> R2 REVISE(1C/2M/3m) -> R3 APPROVED(0C/3M/3m).

Summary: Source-verified Rev-3: Decision-3 drop is correct, PROFILE-FIRST is a hard gate, TB/B4/drop-suppression contracts explicit, C2 executor gate falsifiable with asm-A/B + p<0.05 + documented revert. No CRITICAL remarks. Approved with MAJOR/MINOR items to fold into Wave 0/dev briefs.

MAJOR/MINOR items to fold into Wave-0/dev briefs:

1. [MAJOR] DRAIN-ELISION CORRECTNESS (verify, don't just assume). Decision 2b/2c elides BOTH `system.apply()` AND `world.drain_deferred_hook_queue()` for HAS_DEFERRED==false systems. I verified this is SOUND: `deferred_hook_queue` is only pushed by hooks/observers firing during STRUCTURAL mutation (deferred_master.rs:175/191/207/235/244/253), which a read-only system's `run_unsafe` cannot trigger (no Commands/EntityCommands, read-only UnsafeEcsCell view); and the queue is always brought-to-empty by the previous deferred system's drain (depth-gated, ecs_master.rs:2470-2514). HOWEVER this soundness rests on the invariant 'HAS_DEFERRED==false => run_unsafe enqueues nothing into deferred_hook_queue'. The M1-r1 leaf-audit test asserts the bool VALUES but does NOT assert this behavioral invariant. ADD a behavioral test: a HAS_DEFERRED==false system (e.g. Query+EventWriter) interleaved AFTER a HAS_DEFERRED==true system whose hooks enqueue deferred commands, asserting (a) the deferred commands still apply exactly once and (b) the false-arm skip never strands a non-empty queue. This is the Phase-14b 'silent fire-site' class applied to the SKIP direction.

2. [MAJOR] STALE LINE REFERENCES that the dev/codegen brief will follow literally. tuple_impl.rs: plan cites `()`=`:54` (correct), but tuple `apply`=`:90/196` is WRONG — the working tuple `apply` is at `:149` inside `impl_system_param_tuple!` (macro at `:82`, invocations `:164-175` for arity 1-12); `:196` is the `impl_system_param_tuple_too_large!` STUB macro (arity 13-24, bodies = `const{panic!}`). The OR-fold const must be added to the working macro at `:149`-adjacent AND the unit impl at `:54` inherits the `false` default (no edit). NOT-ADDRESSED CONSEQUENCE: the `too_large` stubs (`:196`) re-implement SystemParam — they MUST inherit the `HAS_DEFERRED` default (do NOT add an override there, or you force const eval of a panicking monomorphization). Confirm the trait default makes the stubs compile untouched. Also schedule.rs apply-window `apply` is at `:686`/drain `:693` and serial `apply` at `:1043`/drain `:1046` (plan's `:686`/`:1043` are right for the apply calls). Fix the tuple_impl refs before Wave 1 codegen.

3. [MAJOR] EXECUTOR-GATE PANIC-SAFETY CONTRACT under-specified. Both executor apply sites UNCONDITIONALLY wrap `apply` in `DeferredScopeGuard::enter()` and there is NO schedule-level catch_unwind (verified schedule.rs:677-693, :1038-1046; the only catch is inside CommandQueue::apply). The plan's pseudocode shows the guard moving INSIDE the `if has_deferred` arm — that is correct (no guard needed when nothing applies), but the plan must state explicitly that for HAS_DEFERRED==false the DeferredScopeGuard is also skipped, and that this cannot change panic-unwind depth accounting (guard only touches TLS depth; skipping enter/drop for a no-deferred system is a balanced no-op). Make the C2 asm-A/B gate assert the false-arm emits NEITHER the guard enter/Drop NOR the drain call, and the true-arm keeps guard+apply+drain byte-identical-modulo-one-branch. Without pinning the guard handling, a dev could leave the guard outside the branch (wasted TLS write per read-only system) or move drain incorrectly.

4. [MINOR] Decision 4 write_row_typed reuses `for_each_data_component_bytes`'s ManuallyDrop discipline but that macro currently re-runs `sort_unstable_by_key` PER CALL (lib.rs:1344) — the plan correctly moves the permutation to once-per-batch via `perm[k]`, but must ensure the NEW typed method does NOT also emit the per-row sort (the whole point). The B1 canonical order must be baked into `perm` at codegen/per-batch, and a debug_assert should confirm `perm` yields canonical-sorted column order to preserve the SBO-B2 contract the byte path enforces at spawn_batch_command.rs:437-439. State this in the Wave-2 brief so the dev doesn't copy the sort into write_row_typed.

5. [MINOR] Wave-0 deliverables include three benches that DO NOT YET EXIST: `comparison_v2::g2c` (cached FunctionSystem), the deferred-heavy 50-Commands scheduler micro (the PRIMARY C2 true-arm gate), and the D6 small-world create-and-spawn micro. The plan names them but the existing bench files (comparison.rs, comparison_v2.rs, profile_spawn*.rs, growth_crossing*.rs) don't contain them. Make Wave-0 Step 0.1/0.2 explicitly include AUTHORING these three benches as a prerequisite deliverable (not just 'profile against them'), else the C2 gate has no harness and Wave 3 cannot run.

6. [MINOR] O3-r1 NT-stores caveat is correctly deferred, but Open-Q2 should also note that spawned rows feed not just the next query but `commit_units_batch`+`fill_ticks_batch` which run IMMEDIATELY after the write loop (spawn_batch_command.rs:465-471) and touch tick arrays in the same cache region — an NT store on the data columns won't pollute ticks (separate pools, SoA), so the NT decision is purely about the data stream vs first-read. Minor: the 'full spawn + first-query' metric should be the gate, as the plan already says; no change needed beyond acknowledging fill_ticks runs before the first query and is unaffected.

