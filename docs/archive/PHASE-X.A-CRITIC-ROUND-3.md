# Phase X.A — Post-Landing Critic Review (Round 3)

This review was run AFTER Phase X.A landed (commits `5499855`→`e2cd17b`),
as an independent audit of the shipped code + plan + results. It is a
**polish backlog**, not a blocker — the feature is landed and
`PHASE-X.A-RESULTS.md` honestly documents the headline outcome (5× target
NOT met on single-component; credible 1.28–1.34× multi-component win under
`-Ctarget-cpu=native`). Verdict: REVISE the plan doc for accuracy + apply a
handful of doc/bench improvements to the landed code.

## Actionable items (priority order)

### C1 [CRITICAL — doc accuracy] Reconcile PLAN §1.2/§8.4/§11.6 with measured reality
The plan still frames "boyko ≥ 5× Bevy" as a hard PASS gate. It was unmet
and was structurally unfalsifiable (Bevy's `fold` override lowers to the
same LLVM reduction; the only cited evidence — orlp.net 21.6× — is
intra-engine `naive` vs `algebraic`, not boyko-vs-Bevy). `RESULTS.md` is
honest; the PLAN is not. **Fix:** rewrite §1.2 row 1 + §8.4 + §11.6 to a
defensible, falsifiable target: "≥1.10× Bevy on a ≥3-column reduction with
`-Ctarget-cpu=native`; parity-or-better on single-component." Either delete
the 5× gate or mark it explicitly aspirational/stretch. State plainly that
single-component reductions are expected at parity (both autovectorize).

### C2 [HIGH — bench fairness] Add a brutally-fair Bevy `for`-body lane to g6
Current g6 compares boyko's hand-written `for` body vs Bevy's
`iter().fold(_, algebraic_add)` — two different idioms. A real Bevy user
writes `for v in query.iter() { acc = algebraic_add(acc, v.0) }`. **Fix:**
add a third single-component lane (Bevy `for`-body, the literal mirror of
the boyko closure body) so the API-shape delta is isolated from the
`fold`-override specialization. Report all three.

### C3 [HIGH — correctness footgun doc] `&mut [T]` writes are invisible to change detection
`Query<&mut T>::for_each_chunk(|s: &mut [T]| ...)` writes through raw
column pointers and never bumps `changed_ticks`. This is the OPPOSITE of
Bevy's default mutable accessor (Bevy's `Mut` bumps ticks). PLAN §7.2's
"same as Bevy" claim is wrong. A user batching a physics integrate over
`&mut Velocity` produces velocities no `Changed<Velocity>` system observes,
with zero signal. **Fix:** (a) strike the "same as Bevy" wording; (b) add a
prominent doc-warning on `for_each_chunk` itself (not just the trait) that
`&mut [T]` writes do not trigger change detection — use `iter_mut`+`Mut<T>`
when tracking is needed. (`_tracked` chunked variant is deferred to 13.X.)

### C4 [HIGH — plan/code divergence] Document the `where Or<F>: QueryFilter` clause
PLAN §5.2 shows `unsafe impl<F: ArchetypalQueryFilter> ArchetypalQueryFilter
for Or<F> {}` — which does NOT compile as written; the shipped code
(`filter.rs:1718-1722`) needs `where Or<F>: QueryFilter` because
`QueryFilter for Or<_>` only exists for tuple `F`. **Fix:** update §5.2 to
the real impl + explain why. Add a trybuild test for a nested-`Or` tick
filter: `Or<(With<A>, Or<(With<B>, Changed<C>)>)>` (current suite covers
only one `Or` level).

### C6 [MEDIUM — ergonomics doc] `par_for_each_chunk`'s `Fn + Sync` accumulator cliff
A closure capturing `&mut acc` compiles under `for_each_chunk` (FnMut) but
NOT under `par_for_each_chunk` (Fn + Sync). The two look like a
sequential/parallel pair but have incompatible capture rules. **Fix:**
explicit doc callout + a one-line worked example (AtomicU32/sharded TLS) on
`par_for_each_chunk`. (Real fix is `par_fold_chunks`, deferred to 13.X.)

### C5 / C7 [MEDIUM — verification debt] Complete the skipped asm check; reconcile plan framing
§11.6 asm inspection was SKIPPED (cargo-show-asm unavailable). The
single-component "boyko 5–11% slower" deficit is unexplained — a slice path
doing zero per-row engine work should not be slower than a per-row state
machine. **Fix:** when `cargo-show-asm` is available, confirm boyko's inner
loop is identical and the deficit is harness noise (not a missed inline or
dispatch tax). Add a 1-archetype-N-row microbench isolating per-archetype
dispatch cost. Reframe PLAN §12/§14 from "ready for developer / no open
questions" to "post-implementation — see RESULTS.md; residuals
X.A.1/X.A.2 open."

### C8 / C9 [LOW]
- C8: `'c` HRTB forecloses "collect all slices for a second pass" (no
  chunked alternative this phase) — one honest sentence in the risk note.
- C9: PAR9 inline gate uses const `MIN_ARCHETYPE_FOR_PARALLEL` while
  `chunk_size` uses `batching.min_batch_size`; they disagree if the user
  overrides `min_batch_size`. Document the intended behavior or align them.

## Preserved positives (verified correct in shipped code)
- All `unsafe` blocks carry specific `// SAFETY:` comments; the
  `column.ptr == buffer_ptr()` → `SIMD_BUFFER_ALIGN`-aligned → `fetch_chunk`
  slice derivation is sound; slice len == `entity_count` not capacity.
- CD4 `&mut T::set_chunk_readonly` is `#[cold] #[inline(never)] panic!`.
- Filter gate sound: `ArchetypalQueryFilter` excludes `Added`/`Changed`;
  tuple/`Or` propagation requires every element archetypal; trybuild-locked.
- Sibling-trait `ChunkedQueryData` (not GAT-on-`QueryData`) is the right
  call — 15-impl additive surface, zero break to custom `QueryData`.
- NCD const-fold elision real: chunk drivers drop the `meta` plumbing.
- Alignment lift correct; `Vec3` per-row-SIMD trap side-stepped
  (column-start alignment only, engine emits no SIMD).
- Inline policy follows principle 7 (no blind `inline(always)`).
- No hot-path allocations / locks / dyn / HashMap.
