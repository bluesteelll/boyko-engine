# Option<&T> / AnyOf<(...)> query data (task #9) — resolved plan

Branch `ecs`, 2026-06-16. P0 query ergonomics: optional, non-filtering query data.
Produced by architect → 2 independent critics (correctness + cross-feature
interactions); this doc folds every CRITICAL/MAJOR finding in as a resolved
decision. It is the implementation spec.

## Shape (sound core — preserved from the design)

- `Option<D>`: `Fetch = OptionFetch<D> { inner: D::Fetch<'w>, matches: bool }`
  (manual `Copy`/`Clone`, mirrors `ReadFetch`); `Item = Option<D::Item>`;
  `matches_component_set = true` (NON-filtering); `aggregate_include = no-op` (no
  required bit); `HAS_DATA_COMPONENT = false`; `NEEDS_CHANGE_DETECTION = D::NCD`;
  `IS_READ_ONLY = D::IS_READ_ONLY`. `ReadOnlyQueryData for Option<D> where D:
  ReadOnlyQueryData` (so `Option<&mut T>`/`Option<Mut<T>>` are rejected from
  `iter()`/`par_iter()` — only `iter_mut()` admits them).
- `AnyOf<(D0, D1, ...)>`: `Fetch = ((D0::Fetch, bool), ...)`; `Item =
  (Option<D0::Item>, ...)`; `matches_component_set = OR of arms`;
  `aggregate_include = no-op`; `HAS_DATA_COMPONENT = false`; `NCD = OR of arms`;
  `IS_READ_ONLY = AND of arms`. ReadOnly only when all arms are.
- **Do NOT populate the `optional` mask** (state.rs `QueryState::matches` treats it
  as an OR-*requirement* `mask.intersects(optional)` — populating it would WRONGLY
  require T present, the opposite of non-filtering). Verified `query_state.rs:295-303`.
- **No existing trait method/const/impl is changed** → existing `&T`/`&mut`/tuple
  hot path is byte-identical (0%-gate by construction). The ONE addition is a
  defaulted const (Decision 4) — additive, no impl churn, cold-only use.

## Decision 1 — gate the inner `set_table` on `matches` (critic C1, the false-precedent fix)

The design's "byte-for-byte OrFetch" justification is **WRONG and must not be
copied**. OrFilter forwards each arm's `set_table` **unconditionally** then computes
`matches` — sound only because its arms are `With`/`Without` (`()` fetch, no-op
set_table) or `Added`/`Changed` (tolerate a NULL tick base). `&T`/`&mut T`/`Ref`/
`Mut::set_table` is the OPPOSITE: it reads `columns[id]` and `debug_assert!(!ptr
.is_null())` (data.rs:383). So for an archetype lacking T an unconditional forward
**panics in debug + stores a NULL base**.

Therefore `Option<D>::set_table_*` MUST:
```
matches = D::matches_component_set(state, &archetype.component_mask());
if matches { D::set_table_*(&mut fetch.inner, state, archetype, [meta]); }
// else: leave fetch.inner at its D::init_fetch NULL-init value (never read).
fetch.matches = matches;
```
`fetch`: `if fetch.matches { Some(D::fetch(&fetch.inner, row)) } else { None }`.
SAFETY is rederived from the gate: `matches==true ⇒ matches_component_set held ⇒
columns[id].ptr non-null ⇒ D::set_table's QD1/QD3 + its internal debug_assert hold;
matches==false ⇒ inner is the NULL-init fetch and is NEVER read (fetch returns
None)`. Strike every "mirrors OrFetch" claim — Option's set_table is the *inverse*
(gated, not unconditional).

## Decision 2 — the four `set_table_*` variants per inner NCD class (critics C2/W3)

`F::NCD` can force the **meta** path even when `D::NCD == false` (e.g.
`Query<Option<&A>, Changed<X>>`). So for an NCD=false inner D, BOTH the meta and the
`_no_meta` `Option` bodies must be REAL (gated-forward), not panics. Exact table:

| inner D | D::NCD | readonly meta | readonly no_meta | mut meta | mut no_meta |
|---|---|---|---|---|---|
| `&T` | false | real gated-fwd | real gated-fwd | n/a (panic backstop, kept) | n/a (panic backstop, kept) |
| `&mut T` | false | panic backstop (kept) | panic backstop (kept) | real gated-fwd | real gated-fwd |
| `Ref<T>` | true | real gated-fwd | fwd→inner cold-panic (unreachable by const-fold) | n/a panic | n/a panic |
| `Mut<T>` | true | n/a panic | n/a panic | real gated-fwd | fwd→inner cold-panic (unreachable) |

Rules: (a) the `matches`-gate wraps the FORWARD, never the panic — the QD4 readonly
backstop-panic on a write-inner must be PRESERVED verbatim (not gated away), so a
future wrong `ReadOnlyQueryData for Option<&mut T>` still fails loud. (b) The
`_no_meta`→inner-cold-panic for `Ref`/`Mut` is unreachable because `Option<Ref/Mut>
::NCD = true` routes the driver (iter.rs:298 `if const { D::NCD || F::NCD }`) to the
meta path. (c) `Option<&T>`/`Option<&mut T>` (NCD=false) DO NOT participate in change
detection (consistent with bare `&T`/`&mut T`).

## Decision 3 — AnyOf arms are sealed (critic W4): only real-component leaves

`AnyOf<(&A, Option<&B>)>` and `AnyOf<((), ...)>` compile under a naive `Di:
QueryData` bound and SILENTLY MATCH THE WHOLE WORLD (an arm whose
`matches_component_set` is unconditionally `true` breaks the OR ≥1-member trim).
Fix: a sealed marker trait
```
pub trait AnyOfArm: QueryData {}      // sealed
impl<T: Component> AnyOfArm for &T {}
impl<T: Component> AnyOfArm for &mut T {}
impl<T: Component> AnyOfArm for Ref<'_, T> {}
impl<T: Component> AnyOfArm for Mut<'_, T> {}
```
The variadic `AnyOf` impl bounds every arm `$D: AnyOfArm`. This compile-rejects
`Option`, `()`, nested `AnyOf`, and tuple arms (none impl `AnyOfArm`) — closing the
OR-break. Mirrors the sealed `OrComposable` bound from BUG-ENABLE-PRE-1.

## Decision 4 — AnyOf + sole enable: fix via `REQUIRES_POST_FILTER_TRIM` (critics C2/W1)

`Query<AnyOf<(&A,&B)>, Enabled<C>>` has `D::HAS_DATA_COMPONENT=false` +
`F::IS_SOLE_SINGLE_ENABLE=true` ⇒ `IS_CANDIDATE_SEEDED=true` ⇒ the candidate-seed
branch SKIPS `post_filter_matched` (state.rs:204-205, 226-233). But AnyOf's ≥1-member
OR-trim lives ONLY in post_filter (via `matches_component_set`). Result: a C-present
archetype with neither A nor B is visited and yields `(None, None)` — a contract
violation, not "harmless".

**Resolution (clean, no compile-reject, no phantom rows):** add a defaulted
`QueryData` const
```
/// Default: false. True iff this data needs a per-archetype post-filter trim
/// (its matches_component_set is not unconditionally true) — only AnyOf (OR).
const REQUIRES_POST_FILTER_TRIM: bool = false;   // DEFAULTED — additive, 0%-gate safe
```
`AnyOf` sets it `true`; `Option` keeps `false` (its matches_component_set is always
true → never trims). Fold it into the candidate-seed predicate:
```
IS_CANDIDATE_SEEDED = F::IS_SOLE_SINGLE_ENABLE && !D::HAS_DATA_COMPONENT
                      && !F::HAS_POSITIVE_ARCHETYPAL && !D::REQUIRES_POST_FILTER_TRIM
```
Then `Query<AnyOf<...>, Enabled<C>>` is NOT candidate-seeded → takes the normal
`update_archetypes` + `post_filter_matched` (AnyOf OR-trim applies) + cull path →
correct: archetypes with (A or B) AND a C-column, per-row enable-filtered. The const
is the ONLY trait addition; it is defaulted (no impl churn), used only in the cold
const-folded shape evaluation (0%-gate: hot path untouched, non-AnyOf queries
unaffected since the default keeps the formula identical). Rationale for a default
(vs the NCD/HAS_DATA_COMPONENT no-default I4 discipline): a false default is SAFE —
it only ever needs flipping for an OR-matching data type, and a future such type
forgetting it affects only the exotic enable-combo, not a common hot path.

## Decision 5 — Option/AnyOf do NOT implement `ChunkedQueryData` (critic C1-interactions)

`for_each_chunk`/`par_for_each_chunk` dispatch through a SEPARATE trait
`ChunkedQueryData` (chunked_data.rs) with `ChunkFetch`/`fetch_chunk -> &[T]` — no
per-archetype `matches`, no per-row gating, no NCD split. Per-row `Option` gating is
incompatible with whole-archetype slice chunking. **Do NOT impl `ChunkedQueryData`
for `Option`/`AnyOf`** → `for_each_chunk` rejects them at compile time (same posture
as `Ref`/`Mut`). Add a `compile_fail` test (`Query<Option<&T>>::for_each_chunk`
rejected). Whole-archetype `Option<&[T]>` chunking, if ever wanted, is a separate
follow-up with its own semantics.

## Decision 6 — re-exports (critic Q4 / the Phase-14b "invisible" class)

`Option` is std (no re-export). `AnyOf` is a NEW public type → re-export it at the
query module's pub site (mirror `data::{Mut, QueryData, ReadOnlyQueryData, Ref}` at
`mod.rs:54`) AND add it to the crate prelude alongside `Ref`/`Mut`. `OptionFetch`/
`AnyOfFetch` are implementation types — re-export only if the public API needs to
name them (likely `pub(crate)`).

## Decision 7 — degenerate combos: legal, documented, test-pinned (critics W3/W5)

Do NOT compile-reject these (over-restricting breaks generic code); doc + test-pin:
- `Query<Option<&A>, Without<A>>` → `Option` is always `None` (Without excludes A).
- `Query<Option<&A>, Changed<A>>`/`Added<A>` → always `Some` (filter trims to A-present);
  confirm the NCD meta-path routing (F::NCD=true forces Option's meta set_table — real
  body per Decision 2).
- `AnyOf<(&A,)>` single arm → Item is `(Option<&A>,)` (always `Some`, bounded to
  A-present) — NOT equivalent to `&A`; clarify in docs.
- `AnyOf<(&A, &A)>` overlapping read+read → legal (no conflict).
- `AnyOf<(&mut A, &A)>` / `(&mut A, &mut A)` → MUST trip the B0002 aliasing detector
  (init_access forwards each arm). Add the test.
- empty `AnyOf<()>` → no impl (trait-not-satisfied) → compile error; trybuild test.

## Decision 8 — access + matching (preserved)

`init_access` forwards to inner D / each arm (declares the read/write — conservative,
correct; `Query<(&mut A, Option<&A>)>` and `Query<(&mut A, AnyOf<(&A,_)>)>` must trip
B0002). Sole `Query<Option<&T>>` / `Query<AnyOf<...>>` have empty include ⇒
`update_archetypes` matches ALL archetypes then `post_filter_matched` trims (Option:
no trim; AnyOf: ≥1-member) — the `Or<F>` cost profile (paid per generation bump, not
per `iter()`). Document the full-world-scan cost in the `AnyOf` doc comment + an
archetype-count-scaling bench note (so it's not later mistaken for a bug). The common
`Query<(&A, Option<&B>)>` is bounded by `&A`'s include (efficient).

## Soundness

No new `unsafe` beyond the gated forwards (each set_table/fetch carries the
gate-derived `// SAFETY:` per Decision 1). The `matches==false` inner fetch is the
NULL-init value, never read. par_iter reads the same per-archetype `matches` (computed
in its set_table) — no new sync.

## Gates

- **0%-gate (sacred):** existing `&T`/`&mut`/`Ref`/`Mut`/tuple/`()` impls UNCHANGED;
  the only trait addition is a defaulted const used in cold const-eval; hot cursor
  untouched. Verify with the `query_iter` criterion bench (same-binary A/B per the
  box's drift caveat) + asm spot-check.
- **Tests:** Option Some/None per archetype; `(&A, Option<&B>)` mixed; `Option<&mut>`
  write; `Option<Ref>`/`Option<Mut>` change detection fires only when present;
  `iter()` rejects `Option<&mut T>` (compile_fail); AnyOf OR-match + ≥1 guarantee;
  `AnyOf+Enabled<C>` yields NO `(None,None)` rows (the Decision-4 regression test);
  AnyOf sealed-arm rejects `AnyOf<(&A, Option<&B>)>`/`AnyOf<((), _)>` (compile_fail);
  empty `AnyOf<()>` compile_fail; `for_each_chunk` rejects Option/AnyOf (compile_fail);
  degenerate combos (Decision 7); aliasing B0002 for `(&mut A, Option<&A>)` and
  `AnyOf<(&mut A, &A)>`; `get` on sole `Option<&T>` for a T-less entity returns
  `Some(None)`; Miri-TB on the gated set_table/fetch paths.
- **Toolchain:** gnu-1.96 (`+stable-x86_64-pc-windows-gnu`, pkg `boyko-ecs`).
