# Phase 12.5 Track B — Critic Round 2

Critique of `docs/PHASE-12.5-QUERY-OPTIMIZATIONS-PLAN.md` (Round 2).
Verdict: **NEEDS-FIX**.

## Round 1 follow-up verification

| Finding | Status | Notes |
|---------|--------|-------|
| C1 — EcsMaster Send+Sync | ✅ ADDRESSED | §0 + §7 + §QV1 + §PHASE9.1-5 consistently reflect Send+Sync. Borrow-checker-gates-the-view framing correct. |
| C2 — query_ref impossible | ✅ ADDRESSED | Option (a) chosen — dropped from v1; deferred to Phase 13. |
| C3 — Cache alloc pattern | ⚠️ PARTIALLY | Constructor pattern correct. Tripwire test scheduled. NEW inconsistency: §1.2 claims 3 ns budget, §6.1 enumerates ~5 ns. Pick one. |
| C4 — SystemMeta::DUMMY | ❌ FAILS | Architect committed to const fn-ify `BitSet::new` etc. — but `BitSet::<T>::new` calls `T::default()`, which is non-const trait method. `const_trait_impl` is unstable. Step 0a will not compile. See C-NEW-1. |
| C5 — Drop ordering | ✅ ADDRESSED | Option (a) chosen — query_state_cache AFTER arena. Miri test registered. |
| C6 — Success criterion | ✅ ADDRESSED | Architect chose hybrid: extracted Opt-B4 + Opt-B5 levers AND amended criterion to `≤ bevy + 5% noise floor`. (But see C-NEW-2 / C-NEW-3 / C-NEW-5 — the extracted levers don't pan out.) |
| I1 — Tree Borrows hygiene | ✅ ADDRESSED | UnsafeCell wrapping. Miri tests registered. |
| I2 — LTO pinning | ✅ ADDRESSED | Phase 8.5 BundleTypeKey pattern verbatim. |
| I3 — Self-referential variant | ✅ ADDRESSED | Collapses naturally per C2 (a). |
| I4 — NCD5 footgun | ✅ ADDRESSED | No default body; every impl declares explicit no-meta variant or panic. |
| I5 — MAX_QUERY_TYPES knob | ✅ ADDRESSED | Cargo feature `big_query_table`. |

## NEW CRITICAL findings

### C-NEW-1. Step 0 const-fn chain CANNOT work — `BitSet<T>::new` calls `T::default()`, which is non-const trait method

**Plan claim**: "BitSet::<u64>::new: trivially const fn (sets one u64 field to 0)."

**Reality** (`boyko_utils/src/bit_mask/bit_set.rs:87-89`):
```rust
pub fn new() -> Self {
    Self { bits: T::default() }  // T::default() is non-const trait method
}
```

`BitInteger: Copy + Default` — `Default` is the standard trait, NOT const-trait.
`const_trait_impl` is unstable in Rust 1.85 / May 2026. Compile-error:
```
error[E0015]: cannot call non-const fn `<u64 as std::default::Default>::default` in constant functions
```

**Fix options**:
- **(a)** Avoid trait dependency: add `pub const fn zero() -> Self where T: ConstZero` or `BitSet<u64>::ZERO: BitSet<u64> = BitSet { bits: 0 }` inherent const. Requires `BitSet` redesign — not "promote 4 fns to const fn".
- **(b)** Fall back to C4 path (c): `OnceLock<SystemMeta>::DUMMY`. Plan rejected this in §0 because it adds 2 ns to cache-hit budget. Accept the cost.

### C-NEW-2. Opt-B5 calls non-existent `Archetype::column_raw_ptr(component_id)` method

`grep -r 'column_raw_ptr' crates/` returns hits only in the plan itself.
The real path (`data.rs:307-309`):
```rust
let column = unsafe { (*archetype).columns.get_unchecked(state.id.0) };
fetch.base = column.ptr as *const T;
```

**Fix**: either (a) enumerate `Archetype::column_raw_ptr` as a new method-to-add in §9 with SAFETY invariant, OR (b) drop Opt-B5 and re-validate asm-level claim against real `columns.get_unchecked(id.0).ptr` access path. The "4-instruction saving" claim in §6.5 is unverified.

### C-NEW-3. Opt-B4 misrepresents non-atomic storage as atomic

`ArchetypeMaster.generation` and `ArchetypeMaster.structural_generation` are plain `ArchetypeGeneration(NonZeroUsize)` fields (`archetype_master.rs:37, 54`). They are **NOT** `AtomicUsize`.

Plan §2.7 B4.2 claims "Two `AtomicUsize`-equivalent fields; combined load is fused atomic read on x86_64 (LLVM lowers tight back-to-back atomic loads to a single `mov`)." There are no atomics here. There is no "fuse two atomic loads" lever.

`combined_generation_snapshot` (§4.7) just returns a tuple of two existing non-atomic loads — identical to what `archetype_generation()` + `structural_generation()` already do. **The method is a no-op rename.**

The "branchless OR'd dirty check" lever in §6.4 may be real but is generic compiler optimisation that LLVM may already perform. The "~0.2 µs at 10k calls" is unsubstantiated. Bench shape misalignment: bench has 10k inner iterations within ONE query call, not 10k separate query calls.

**Fix**: drop the "fused atomic load" framing. Honestly note Opt-B4's potential lever is purely branchless OR vs `||` — needs `cargo asm` dump to confirm LLVM doesn't already do this. At single-archetype workload (1 outer trip), saves ~1 cycle per query call — zero impact on the 10k bench.

### C-NEW-4. Cache-hit budget internally inconsistent

§1.2 row 3: target ≤ 3 ns. Phase 8.5 anchor 226 ps cited.
§6.1 enumerates 6 steps totalling ~5 ns including `state.update(...)`.

Phase 8.5's 226 ps is for a single `OnceLock::get()` — NOT for the full `query` path with `state.update`. §6.1 figure looks defensible; §1.2 row 3 is wishful.

**Fix**: reconcile. Either target ~5 ns (honest), or separate "lookup cost" from "full query cost" and cite which.

### C-NEW-5. Opt-B5 LTO assembly claim and bench update are circularly justified

§6.5: "Opt-B5's saving comes from `fetch::<&T>` per-row collapsing — LLVM should fuse this anyway; assembly check in Step E2 will confirm; if it doesn't materialise, Opt-B5's benefit will be honestly recorded."
§F1: commits to *replacing* `g2_boyko_query_iter_10k` with `iter_single_read` for the gate at ≤ 7.25 µs.

If LLVM already fuses, Opt-B5 produces zero speedup. But the gate uses `iter_single_read` — so "no improvement" makes the gate trivially satisfied by Opt-B1 alone, hiding Opt-B5's null contribution.

**Fix**: Keep BOTH benches (generic `iter()` apples-to-apples with Bevy + `iter_single_read` for specialised path). Define Step E1 acceptance: measurable ≥ 100 ns improvement on `p2_boyko_single_read_10k` vs `p2_boyko_direct_api_10k`. If not measurable, Opt-B5 is dropped, not silently retained.

## NEW IMPORTANT findings

### I-NEW-1. NCD5 LOC budget under-estimated

12 tuple arities × 4 macros × 2 new methods = ~96 mechanical edits in `data.rs` alone, plus filter, plus leaf impls. §1.2 budget of 800-1200 production LOC is tight. Phase 8.5 similar work landed ~1500 production LOC.

**Fix**: revise budget to ~1500-2000.

### I-NEW-2. `bundle_archetype_cache` field ordering interaction with `query_state_cache` placement

§10.1 places `bundle_archetype_cache` BEFORE `change_tick`/`arena`, and `query_state_cache` AFTER `arena`. Existing `ecs_master.rs:111-134` documents `bundle_archetype_cache` rationale.

**Fix**: explicitly cite that `bundle_archetype_cache` is unaffected; document the change is purely the *addition* of `query_state_cache` after `arena`.

### I-NEW-3. Cache slot reborrow chain produces `&mut` retag per call

§5.1: `let state_mut: &mut QueryDataState<D, F> = &mut *(*cell_ptr.as_ptr()).get();`
This mints `&mut QueryDataState` on every call. Sound under `&mut self` gate, but the §0 I1 wording "no `&mut` retag" overstates.

**Fix**: rewrite §0 I1 entry — "the `&mut` retag now derives from `&mut self`'s unique provenance, not from a raw `Box::leak + as_mut`". Add Miri test that calls `query()` 1000× in a row.

### I-NEW-4. MAX_CHANGE_AGE usage for `Ref<T>` direct calls breaks semantics

For `Query<Ref<T>>` direct calls, plan synthesises `last_run = current - MAX_CHANGE_AGE`. This means every direct call observes "every row Changed since last frame" silently — broken semantics.

**Fix options**: (a) Compile-error: `where (D, F): NoCDetect`. (b) Runtime panic: "direct API does not support change-detection filters; use Query<D, F> inside a system". (c) Track `last_run` per-(D, F) in cache slot — adds state but correct.

### I-NEW-5. QueryView Send+Sync asymmetry with existing Query<'w, 's, D, F>

`QueryView<'w, D, F>: Send + Sync` asserted. Audit existing `Query<'w, 's, D, F>` SystemParam. Document parity or call out asymmetry.

**Fix**: add `static_assertions::assert_impl_all!(QueryView<'static, &Pos, ()>: Send, Sync)`.

## NEW OPTIONAL

- O-NEW-1. §2.7 B4.1 Bevy `tables_id_count` u32 load claim — cite or drop.
- O-NEW-2. `combined_generation_snapshot` name suggests packed u64 — rename or actually pack.
- O-NEW-3. `offset_of!` test methodology relies on `repr(Rust)` field order — fragile across toolchains. Use Drop-order observation test.
- O-NEW-4. §0 C3 slot size "~24-32 B" — pin at `<= 24` matching Phase 8.5 tripwire. Recompute 24 KB footprint.

## VERDICT

**NEEDS-FIX**

## RATIONALE

Round 2 successfully resolves 5 of 6 Round 1 criticals (C1, C2, C5, C6, partially C3) and all 5 importants. However:

- **C4 fails outright (C-NEW-1)**: const-fn migration cannot work — `BitSet<T>::new` calls `T::default()` which is non-const trait method.
- **C-NEW-2** (Opt-B5 fictional API), **C-NEW-3** (Opt-B4 misrepresents non-atomic as atomic), **C-NEW-4** (3 ns vs 5 ns contradiction), **C-NEW-5** (Opt-B5 circular justification) are new criticals from Round 2's lever-extraction work.

The architect was forced to invent levers (Opt-B4, Opt-B5) to satisfy the "≥1.10× Bevy" gate from C6, but the levers don't survive scrutiny:
- Opt-B4: claims atomic-load fusion. Storage is non-atomic. No-op rename.
- Opt-B5: claims `column_raw_ptr` method. Doesn't exist. Real path is identical instruction count.

**Recommendation**: Round 3 should honestly amend the umbrella criterion for the query iter bench to "≥ Bevy parity" (closing 0.88× loss to ~1.00×), drop the fictional Opt-B4/B5 levers, fall back to OnceLock<SystemMeta> for C4, and reconcile the 3 ns vs 5 ns budget. This gives a defensible, honest plan that closes the gap without overpromising.
